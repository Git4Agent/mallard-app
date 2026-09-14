// Synthetic homes only. Included in service::tests.
fn save_child_push(repo: &LocalStateRepository, provider: &str, enabled: bool) {
    let mut settings = mallard_infrastructure::app_settings::load_app_settings(repo).unwrap();
    match provider {
        "codex" => settings.push_codex_children = enabled,
        "claude" => settings.push_claude_children = enabled,
        _ => unreachable!(),
    }
    mallard_infrastructure::app_settings::save_app_settings(repo, &settings).unwrap();
}

fn child_settings_fixture(
    home: &Path,
    root: &Path,
    provider: &str,
    parent: &str,
    count: usize,
) -> (PathBuf, Vec<PathBuf>) {
    if provider == "claude" {
        return write_claude_owned_fixture(home, root, parent, count);
    }
    let ids = (0..count)
        .map(|n| format!("019fb002-0000-7000-8000-{n:012x}"))
        .collect::<Vec<_>>();
    let keys = (0..count).map(|n| format!("start{n}")).collect::<Vec<_>>();
    let references = keys
        .iter()
        .zip(&ids)
        .map(|(k, id)| (k.as_str(), id.as_str()))
        .collect::<Vec<_>>();
    let main = write_owned_rollout(home, root, parent, None, &references);
    let children = ids
        .iter()
        .map(|id| write_owned_rollout(home, root, id, Some(parent), &[]))
        .collect();
    (main, children)
}

fn push_child_settings(
    service: &ProjectSyncService<'_>,
    project: &LocalProjectRegistration,
    storage: &StorageId,
) -> (usize, u64) {
    let review = service
        .prepare_push_review(&project.local_project_id, storage)
        .unwrap();
    let mut uploaded = (0, 0);
    let result = service
        .push_reviewed_with_progress(
            &project.local_project_id,
            storage,
            PushApproval::recommended(&review, storage),
            &mut |progress| uploaded = (progress.index, progress.fetched_bytes),
            &mut |_, _| {},
        )
        .unwrap();
    assert!(result.success, "{} {:?}", result.message, result.results);
    uploaded
}

#[test]
fn pulled_children_excluded_hides_agent_cards_until_children_are_pulled() {
    for provider in ["codex", "claude"] {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage = StorageId::parse("child-viewer").unwrap();
        add_local_storage(&repo, &storage, &temp.path().join("store"));
        let source = register(&repo);
        let root = temp.path().join("source");
        let home = temp.path().join("source-provider");
        let bind = |project: &LocalProjectRegistration, root: &Path, home: &Path| {
            if provider == "codex" {
                bind_and_link_test_replica(&repo, project, &storage, root, home);
            } else {
                bind_and_link_claude_replica(&repo, project, &storage, root, home);
            }
        };
        bind(&source, &root, &home);
        let (main, _) = child_settings_fixture(
            &home,
            &root,
            provider,
            "019fb002-0000-7000-8000-00000000cc01",
            3,
        );
        // The transport fixture records Claude results only. Add the parent
        // calls so the viewer has agent cards to hide.
        if provider == "claude" {
            let bytes = std::fs::read_to_string(&main).unwrap();
            let mut records = Vec::new();
            for line in bytes.lines() {
                let mut record: serde_json::Value = serde_json::from_str(line).unwrap();
                if record.get("toolUseResult").is_some() {
                    let call = record["message"]["content"][0]["tool_use_id"].clone();
                    let task = format!("task-{}", call.as_str().unwrap());
                    records.push(serde_json::json!({
                        "type":"assistant", "uuid":task, "parentUuid":record["parentUuid"],
                        "sessionId":record["sessionId"], "cwd":root,
                        "message":{"role":"assistant", "content":[
                            {"type":"tool_use", "id":call, "name":"Task",
                             "input":{"description":"Fixture child", "prompt":"Inspect fixture"}}
                        ]}
                    }));
                    record["parentUuid"] = serde_json::json!(task);
                }
                records.push(record);
            }
            std::fs::write(
                &main,
                records
                    .iter()
                    .map(|record| format!("{record}\n"))
                    .collect::<String>(),
            )
            .unwrap();
        } else {
            append_owned_message(&main, "after-agents");
            use std::io::Write;
            let bytes = std::fs::read_to_string(&main).unwrap();
            let event = serde_json::json!({
                "ordinal":bytes.lines().count(), "timestamp":"2026-09-08T00:02:00Z",
                "type":"event_msg", "payload":{"type":"user_message", "message":"Inspect the fixture"}
            });
            writeln!(
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&main)
                    .unwrap(),
                "{event}"
            )
            .unwrap();
        }
        let target = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "target".into(),
                repository_fingerprint: source.repository_fingerprint.clone(),
                bundle_id: Some(source.bundle_id.clone()),
            },
        )
        .unwrap();
        bind(
            &target,
            &temp.path().join("target"),
            &temp.path().join("target-provider"),
        );
        let service = ProjectSyncService::new(&repo);
        for (phase, enabled) in [false, true, false].into_iter().enumerate() {
            // An unchanged parent can remain synced when children are switched
            // off. Advance the parent to require an actual replacement Pull.
            if phase == 2 {
                if provider == "codex" {
                    append_owned_message(&main, "parent-after-policy-change");
                } else {
                    use std::io::Write;
                    let bytes = std::fs::read_to_string(&main).unwrap();
                    let last: serde_json::Value =
                        serde_json::from_str(bytes.lines().last().unwrap()).unwrap();
                    let next = serde_json::json!({"type":"user", "uuid":"parent-after-policy-change",
                        "parentUuid":last["uuid"], "sessionId":last["sessionId"], "cwd":root,
                        "message":{"role":"user", "content":"Continue the fixture"}});
                    writeln!(
                        std::fs::OpenOptions::new()
                            .append(true)
                            .open(&main)
                            .unwrap(),
                        "{next}"
                    )
                    .unwrap();
                }
            }
            save_child_push(&repo, provider, enabled);
            push_child_settings(&service, &source, &storage);
            restore_child_settings(&service, &repo, &target, &storage);
            // Missing optional fields must keep native agent cards visible.
            let binding_path = repo.conversation_bindings_path();
            let mut saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&binding_path).unwrap()).unwrap();
            for row in saved["bindings"].as_array_mut().unwrap() {
                if row["local_project_id"].as_str() == Some(source.local_project_id.as_str()) {
                    row.as_object_mut()
                        .unwrap()
                        .remove("pulled_children_excluded");
                }
            }
            std::fs::write(&binding_path, serde_json::to_vec(&saved).unwrap()).unwrap();
            // Reopen local state: viewing must not depend on the Pull process
            // or on reading storage. Inspect both native and imported copies.
            let reopened =
                LocalStateRepository::from_home_dir(repo.root().parent().unwrap()).unwrap();
            let viewer = ProjectSyncService::new(&reopened);
            let bindings = reopened.load_conversation_bindings().unwrap();
            for project in [&source, &target] {
                let binding = bindings
                    .bindings
                    .iter()
                    .find(|row| row.local_project_id == project.local_project_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "missing binding: provider={provider} enabled={enabled} project={}",
                            project.display_name
                        )
                    });
                let excluded = project.local_project_id == target.local_project_id && !enabled;
                assert_eq!(
                    binding.pulled_children_excluded,
                    excluded,
                    "provider={provider} enabled={enabled} project={} installed_children={}",
                    project.display_name,
                    binding.installed_subagents.len()
                );
                let resource = conversation_resource_id(
                    &ProviderKey::parse(provider).unwrap(),
                    &binding.thread_id,
                )
                .unwrap();
                let detail = viewer
                    .get_project_session_detail(&project.local_project_id, &resource)
                    .unwrap();
                assert_eq!(
                    detail.event_counts.agent_calls.value,
                    Some(if excluded { 0 } else { 3 }),
                    "{provider}"
                );
                for kinds in [
                    BTreeSet::from([SessionTimelineEventKind::AgentCall]),
                    BTreeSet::from([
                        SessionTimelineEventKind::Prompt,
                        SessionTimelineEventKind::AgentCall,
                    ]),
                ] {
                    let mut cursor = None;
                    let mut events = Vec::new();
                    loop {
                        let page = viewer
                            .get_project_session_timeline_page(
                                &project.local_project_id,
                                &resource,
                                ProjectSessionTimelineRequest {
                                    snapshot_id: detail.snapshot_id.clone(),
                                    cursor,
                                    limit: Some(1),
                                    included_event_kinds: kinds.clone(),
                                    included_tool_categories: BTreeSet::new(),
                                    turn_id: None,
                                },
                            )
                            .unwrap();
                        assert!(page.next_cursor.is_none() || !page.events.is_empty());
                        events.extend(page.events);
                        cursor = page.next_cursor;
                        if cursor.is_none() {
                            break;
                        }
                        assert!(events.len() < 20, "pagination did not terminate");
                    }
                    assert_eq!(
                        events
                            .iter()
                            .filter(|event| event.kind == SessionTimelineEventKind::AgentCall)
                            .count(),
                        if excluded { 0 } else { 3 }
                    );
                    if kinds.contains(&SessionTimelineEventKind::Prompt) {
                        assert!(
                            events
                                .iter()
                                .any(|event| event.kind == SessionTimelineEventKind::Prompt),
                            "provider={provider} enabled={enabled} excluded={excluded} counts={:?}",
                            detail.event_counts
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn child_settings_stamp_scope_and_pull_lifecycle() {
    use std::io::Write;
    for provider in ["codex", "claude"] {
        for count in [0, 8] {
            let temp = tempfile::tempdir().unwrap();
            let repo = repository(&temp);
            let storage = StorageId::parse("child-settings").unwrap();
            add_local_storage(&repo, &storage, &temp.path().join("store"));
            let source = register(&repo);
            let root = temp.path().join("source");
            let home = temp.path().join("provider");
            let bind = |project: &LocalProjectRegistration, root: &Path, home: &Path| {
                if provider == "codex" {
                    bind_and_link_test_replica(&repo, project, &storage, root, home);
                } else {
                    bind_and_link_claude_replica(&repo, project, &storage, root, home);
                }
            };
            bind(&source, &root, &home);
            let parent = "019fb002-0000-7000-8000-00000000aa01";
            let (main, children) = child_settings_fixture(&home, &root, provider, parent, count);
            let service = ProjectSyncService::new(&repo);
            let inspect = || {
                storage_engine(&repo, &storage)
                    .unwrap()
                    .1
                    .inspect(&source.bundle_id)
                    .unwrap()
            };
            let review = || {
                service
                    .prepare_push_review(&source.local_project_id, &storage)
                    .unwrap()
            };
            let check_cloud = |stamp, expected_count| {
                let snapshot = inspect();
                let thread = snapshot.manifest.all_conversations().next().unwrap().2;
                assert_eq!(thread.wire_children, stamp);
                assert_eq!(
                    thread
                        .files
                        .iter()
                        .filter(|file| matches!(
                            file.role,
                            ConversationFileRole::SubagentTranscript { .. }
                        ))
                        .count(),
                    expected_count
                );
            };
            push_child_settings(&service, &source, &storage);
            check_cloud(ConversationWireChildren::On, count);
            let cloud_id = inspect()
                .manifest
                .all_conversations()
                .next()
                .unwrap()
                .1
                .clone();
            let target = register_local_project_with_repository(
                &repo,
                RegisterLocalProjectRequest {
                    display_name: "target".into(),
                    repository_fingerprint: source.repository_fingerprint.clone(),
                    bundle_id: Some(source.bundle_id.clone()),
                },
            )
            .unwrap();
            bind(
                &target,
                &temp.path().join("target"),
                &temp.path().join("target-provider"),
            );
            if count == 8 {
                restore_child_settings(&service, &repo, &target, &storage);
            }
            let remote = (count == 8).then(|| {
                let server = mallard_infrastructure::storage_core::dropbox_store::mock::MockDropboxServer::new();
                let engine = BundleEngine::open(server.store.clone(), storage.clone()).unwrap();
                replay_child_wire_to_dropbox(&repo, &storage, &inspect(), &engine);
                server.take_upload_bodies();
                (server, engine)
            });

            // Baseline and retention gate: a parent append must not resend unchanged children.
            if provider == "codex" {
                append_owned_message(&main, "main-next");
            } else {
                let previous = if count == 0 {
                    "u1".to_string()
                } else {
                    format!("result{}", count - 1)
                };
                writeln!(std::fs::OpenOptions::new().append(true).open(&main).unwrap(), "{}",
                    serde_json::json!({"type":"user","uuid":"main-next","parentUuid":previous,"sessionId":parent,
                        "cwd":root,"message":{"role":"user","content":"main append"}})).unwrap();
            }
            let start = Instant::now();
            let submitted = push_child_settings(&service, &source, &storage);
            println!("child-settings parent-append provider={provider} children={count} submitted={submitted:?} elapsed_ms={:.2}", start.elapsed().as_secs_f64()*1000.0);
            assert_eq!(
                submitted.0, 1,
                "unchanged child objects must be retained: {provider}"
            );
            if let Some((server, engine)) = &remote {
                let snapshot = inspect();
                let child_hashes: BTreeSet<_> = snapshot
                    .manifest
                    .all_files()
                    .filter(|(path, _)| path.as_str().contains("/subagents/"))
                    .map(|(_, entry)| entry.sha256.clone())
                    .collect();
                let counters = replay_child_wire_to_dropbox(&repo, &storage, &snapshot, engine);
                let bodies = server.take_upload_bodies();
                let child_bytes: usize = bodies
                    .iter()
                    .filter(|(hash, _)| child_hashes.contains(hash))
                    .map(|(_, len)| len)
                    .sum();
                assert_eq!(child_bytes, 0);
                assert_eq!(counters, submitted);
                println!("child-settings Dropbox parent-append provider={provider} submitted={counters:?} child_http_bytes={child_bytes}");
            }

            if let Some(child) = children.first() {
                if provider == "codex" {
                    append_owned_message(child, "child-growth");
                } else {
                    append_claude_owned_child(child, parent, "agent0", "child-growth", "ca1");
                }
                let submitted = push_child_settings(&service, &source, &storage);
                println!("child-settings child-append provider={provider} children={count} submitted={submitted:?}");
                assert_eq!(
                    submitted.0, 1,
                    "only the changed child is submitted: {provider}"
                );
                if let Some((server, engine)) = &remote {
                    let snapshot = inspect();
                    let child_hashes: BTreeSet<_> = snapshot
                        .manifest
                        .all_files()
                        .filter(|(path, _)| path.as_str().contains("/subagents/"))
                        .map(|(_, entry)| entry.sha256.clone())
                        .collect();
                    let counters = replay_child_wire_to_dropbox(&repo, &storage, &snapshot, engine);
                    let bodies = server.take_upload_bodies();
                    let child_bodies: Vec<_> = bodies
                        .iter()
                        .filter(|(hash, _)| child_hashes.contains(hash))
                        .collect();
                    assert_eq!(child_bodies.len(), 1);
                    assert_eq!(child_bodies[0].1 as u64, counters.1);
                    println!("child-settings Dropbox child-append provider={provider} submitted={counters:?} child_http_bytes={}", child_bodies[0].1);
                }
            }
            // On cloud, Off local: content stays synced, export stamp permits Publish.
            save_child_push(&repo, provider, false);
            let off_review = review();
            let row = &off_review.thread_comparison.as_ref().unwrap().entries[0];
            assert_eq!(row.state, ThreadSyncState::Synced, "{provider}: {row:?}");
            assert_eq!(row.push_decision.action, ConversationReviewAction::Publish);
            assert!(row.subagent_selection.omitted);
            assert!(row.subagent_selection.keys.is_empty());
            push_child_settings(&service, &source, &storage);
            check_cloud(ConversationWireChildren::Off, 0);
            assert_eq!(
                *inspect().manifest.all_conversations().next().unwrap().1,
                cloud_id
            );
            // Cloud Off ignores local child changes; On Push still exports those children.
            if let Some(child) = children.first() {
                if provider == "codex" {
                    append_owned_message(child, "child-next");
                } else {
                    append_claude_owned_child(
                        child,
                        parent,
                        "agent0",
                        "child-next",
                        "child-growth",
                    );
                }
            }
            for enabled in [false, true] {
                save_child_push(&repo, provider, enabled);
                let current = review();
                let row = &current.thread_comparison.as_ref().unwrap().entries[0];
                assert_eq!(row.state, ThreadSyncState::Synced, "{provider}: {row:?}");
                assert_eq!(
                    row.subagent_selection.keys.len(),
                    if enabled { count } else { 0 }
                );
                assert_eq!(row.push_decision.selectable, enabled);
            }
            repo.mutate_config(|config| {
                for link in config
                    .links
                    .iter_mut()
                    .filter(|link| link.local_project_id == source.local_project_id)
                {
                    for checkpoint in link
                        .conversation_checkpoints
                        .values_mut()
                        .flat_map(|scope| scope.values_mut())
                    {
                        checkpoint.accepted = None;
                        checkpoint.accepted_local_tail = None;
                        checkpoint.accepted_subagents.clear();
                    }
                }
                Ok(())
            })
            .unwrap();
            assert_eq!(
                review().thread_comparison.as_ref().unwrap().entries[0].state,
                ThreadSyncState::Synced,
                "Off adoption: {provider}"
            );
            // A's On Push includes its own children despite the Off comparison scope.
            push_child_settings(&service, &source, &storage);
            check_cloud(ConversationWireChildren::On, count);
            save_child_push(&repo, provider, false);
            push_child_settings(&service, &source, &storage);
            save_child_push(&repo, provider, true);
            // B keeps On and pulls A's Off snapshot. It has no restored children.
            restore_child_settings(&service, &repo, &target, &storage);
            assert_eq!(
                mallard_infrastructure::app_settings::load_app_settings(&repo)
                    .unwrap()
                    .wire_children(&ProviderKey::parse(provider).unwrap()),
                ConversationWireChildren::On
            );
            let stamp_started = Instant::now();
            let submitted = push_child_settings(&service, &target, &storage);
            println!("child-settings empty-On-stamp provider={provider} parent_source_children={count} submitted={submitted:?} review_apply_ms={:.2}", stamp_started.elapsed().as_secs_f64()*1000.0);
            check_cloud(ConversationWireChildren::On, 0);
        }
    }
}

// Replay the production wire files through the real Dropbox adapter. Reuse the
// child's production placement rule; measure the loopback server's HTTP bodies.
fn replay_child_wire_to_dropbox(
    repo: &LocalStateRepository,
    storage: &StorageId,
    source: &BundleSnapshot,
    engine: &BundleEngine<mallard_infrastructure::storage_core::dropbox_store::DropboxObjectStore>,
) -> (usize, u64) {
    let observed = engine
        .inspect_optional_with_token(&source.manifest.bundle.bundle_id)
        .unwrap();
    let previous = observed.as_ref().map(|_| {
        engine
            .inspect(&source.manifest.bundle.bundle_id)
            .unwrap()
            .manifest
    });
    let local = storage_engine(repo, storage).unwrap().1;
    let mut bytes = local.byte_source(source, None);
    let mut scopes = BTreeMap::new();
    let mut pins = BTreeMap::new();
    for (key, scope) in &source.manifest.scopes {
        let old = previous.as_ref().and_then(|m| m.scope(key));
        pins.insert(
            key.clone(),
            old.map(|s| (s.revision, s.content_sha256.clone()))
                .unwrap_or_else(mallard_infrastructure::bundle_engine::absent_scope_pin),
        );
        let mut output = ScopePublishInput {
            recipe: scope.recipe.clone(),
            captured_with: scope.captured_with.clone(),
            conversations: scope.conversations.clone(),
            resources: scope.resources.clone(),
            directories: scope.directories.clone(),
            tombstones: scope.tombstones.clone(),
            ..Default::default()
        };
        for (path, entry) in &scope.files {
            let captured = CapturedBundleFile {
                resource_id: entry.resource_id.clone(),
                bytes: bytes
                    .read_bounded(path, 1024 * 1024, "wire replay")
                    .unwrap()
                    .into(),
                source_mtime: entry.source_mtime,
                mode: entry.mode,
            };
            match PublishedFile::from_child_capture(captured, old.and_then(|s| s.files.get(path))) {
                PublishedFile::Retained(entry) => {
                    output.retained_files.insert(path.clone(), entry);
                }
                PublishedFile::Captured(file) => {
                    output.captured_files.insert(path.clone(), file);
                }
            }
        }
        scopes.insert(key.clone(), output);
    }
    let mut submitted = (0, 0);
    engine
        .publish_with_progress(
            PublishBundleRequest {
                identity: source.manifest.bundle.clone(),
                published_by_origin: source.manifest.published_by_origin.clone(),
                writable_scopes: scopes,
                reviewed_scope_pins: pins,
                expected_head: observed
                    .map(|(snapshot, token)| PublishExpectation::Match(Box::new(snapshot), token))
                    .unwrap_or(PublishExpectation::Absent),
                updated_at: source.manifest.updated_at,
            },
            &mut |p| submitted = (p.index, p.fetched_bytes),
            &mut |_, _| {},
        )
        .unwrap();
    submitted
}

#[test]
fn child_settings_stale_batch_and_child_error_scope() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("child-settings-mixed").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let project = register(&repo);
    let root = temp.path().join("project");
    let codex_home = temp.path().join("codex");
    let claude_home = temp.path().join("claude");
    std::fs::create_dir_all(claude_home.join("projects/source")).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let claude = ProviderKey::parse("claude").unwrap();
    let profiles = BTreeMap::from([
        (codex_key(), add_profile(&repo, &codex_key(), &codex_home)),
        (claude.clone(), add_profile(&repo, &claude, &claude_home)),
    ]);
    save_project_binding_with_repository(
        &repo,
        SaveProjectBindingRequest {
            local_project_id: project.local_project_id.clone(),
            project_root: root.to_string_lossy().into(),
            profile_ids: profiles,
            expected_revision: None,
        },
    )
    .unwrap();
    save_project_link_with_repository(
        &repo,
        SaveProjectLinkRequest {
            local_project_id: project.local_project_id.clone(),
            storage_id: storage.clone(),
            pinned: true,
        },
    )
    .unwrap();
    let parent = "019fb002-0000-7000-8000-00000000aa01";
    let (codex_main, codex_children) =
        child_settings_fixture(&codex_home, &root, "codex", parent, 1);
    let (_, claude_children) = child_settings_fixture(&claude_home, &root, "claude", parent, 1);
    let service = ProjectSyncService::new(&repo);
    let review = || {
        service
            .prepare_push_review(&project.local_project_id, &storage)
            .unwrap()
    };
    let approved = review();
    assert_eq!(
        approved.thread_comparison.as_ref().unwrap().entries.len(),
        2
    );
    save_child_push(&repo, "codex", false);
    let result = service
        .push_reviewed(
            &project.local_project_id,
            &storage,
            PushApproval::recommended(&approved, &storage),
        )
        .unwrap();
    assert!(
        result
            .results
            .iter()
            .any(|row| row.message.as_deref() == Some(owned_subagents::PUSH_CHANGED)),
        "{result:?}"
    );
    let snapshot = storage_engine(&repo, &storage)
        .unwrap()
        .1
        .inspect(&project.bundle_id)
        .unwrap();
    assert_eq!(snapshot.manifest.all_conversations().count(), 1);
    assert_eq!(
        snapshot
            .manifest
            .all_conversations()
            .next()
            .unwrap()
            .2
            .provider,
        claude
    );
    push_child_settings(&service, &project, &storage);
    // Off cloud ignores a missing child. On outgoing is still blocked by missing evidence.
    let codex_bytes = std::fs::read(&codex_children[0]).unwrap();
    std::fs::remove_file(&codex_children[0]).unwrap();
    for enabled in [false, true] {
        save_child_push(&repo, "codex", enabled);
        let current = review();
        let row = current
            .thread_comparison
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|row| row.provider.as_str() == "codex")
            .unwrap();
        assert_eq!(row.state, ThreadSyncState::Synced, "{row:?}");
        assert!(!row.push_decision.selectable, "{row:?}");
    }
    std::fs::write(&codex_children[0], codex_bytes).unwrap();
    assert!(
        review()
            .thread_comparison
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|row| row.provider.as_str() == "codex")
            .unwrap()
            .push_decision
            .selectable
    );
    // On cloud keeps child failures visible even when outgoing is Off.
    save_child_push(&repo, "claude", false);
    std::fs::write(&claude_children[0], b"invalid json\n").unwrap();
    let current = review();
    let row = current
        .thread_comparison
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .find(|row| row.provider.as_str() == "claude")
        .unwrap();
    assert_eq!(row.state, ThreadSyncState::Unknown, "{row:?}");
    assert!(!row.push_decision.selectable);
    // Main failures are never hidden by Off.
    save_child_push(&repo, "codex", false);
    std::fs::write(codex_main, b"invalid json\n").unwrap();
    assert!(review()
        .thread_comparison
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .filter(|row| row.provider.as_str() == "codex")
        .all(|row| !row.push_decision.selectable));
}

#[test]
#[ignore = "Prepare an isolated native UI fixture under an explicit temporary root"]
fn child_settings_native_ui_fixture() {
    let root = PathBuf::from(std::env::var("MALLARD_CHILD_SETTINGS_QA_ROOT").unwrap());
    let root = std::fs::canonicalize(root).unwrap();
    assert!(root.starts_with("/private/tmp"));
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("mallard-children-qa-"));
    let repo = LocalStateRepository::from_home_dir(root.join("home")).unwrap();
    let storage = StorageId::parse("children-qa").unwrap();
    add_local_storage(&repo, &storage, &root.join("store"));
    let project = register(&repo);
    let project_root = root.join("project");
    let codex_home = root.join("codex");
    let claude_home = root.join("claude");
    std::fs::create_dir_all(&project_root).unwrap();
    std::fs::create_dir_all(claude_home.join("projects/source")).unwrap();
    let claude = ProviderKey::parse("claude").unwrap();
    let profiles = BTreeMap::from([
        (codex_key(), add_profile(&repo, &codex_key(), &codex_home)),
        (claude.clone(), add_profile(&repo, &claude, &claude_home)),
    ]);
    save_project_binding_with_repository(
        &repo,
        SaveProjectBindingRequest {
            local_project_id: project.local_project_id.clone(),
            project_root: project_root.to_string_lossy().into(),
            profile_ids: profiles,
            expected_revision: None,
        },
    )
    .unwrap();
    save_project_link_with_repository(
        &repo,
        SaveProjectLinkRequest {
            local_project_id: project.local_project_id.clone(),
            storage_id: storage.clone(),
            pinned: true,
        },
    )
    .unwrap();
    for provider in ["codex", "claude"] {
        let home = if provider == "codex" {
            &codex_home
        } else {
            &claude_home
        };
        child_settings_fixture(
            home,
            &project_root,
            provider,
            "019fb002-0000-7000-8000-00000000aa01",
            8,
        );
    }
    push_child_settings(&ProjectSyncService::new(&repo), &project, &storage);
}

fn restore_child_settings(service: &ProjectSyncService<'_>, repo: &LocalStateRepository, project: &LocalProjectRegistration, storage: &StorageId) {
    let bindings = repo.load_bindings().unwrap();
    let plan = service.plan_bundle_restore(storage, &project.bundle_id, bindings.active_for(&project.local_project_id).unwrap()).unwrap();
    let restored = service.apply_bundle_restore(&plan, plan.conversation_review.iter().filter_map(|row| row.action_id.clone()).collect()).unwrap();
    assert!(restored.success, "{restored:?}");
}
