// Synthetic provider homes only. Included in service::tests.
fn session_push_review(
    service: &ProjectSyncService<'_>,
    project: &LocalProjectRegistration,
    storage: &StorageId,
    resource_id: &ResourceId,
) -> PushReviewContext {
    service
        .prepare_push_review_with_phases(
            &project.local_project_id,
            storage,
            PushScope::Session {
                resource_id: resource_id.clone(),
            },
            &mut |_, _| {},
        )
        .unwrap()
}

#[test]
fn session_push_limits_selection_and_acceptance_to_the_parent_and_approved_children() {
    for provider in ["codex", "claude"] {
        for children in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let repo = repository(&temp);
            save_child_push(&repo, provider, children);
            let storage = StorageId::parse("session-push").unwrap();
            add_local_storage(&repo, &storage, &temp.path().join("store"));
            let project = register(&repo);
            let root = temp.path().join("project");
            let home = temp.path().join("provider");
            if provider == "codex" {
                bind_and_link_test_replica(&repo, &project, &storage, &root, &home);
            } else {
                bind_and_link_claude_replica(&repo, &project, &storage, &root, &home);
            }
            let parent = "019fb002-0000-7000-8000-00000000aa01";
            let sibling = "019fb002-0000-7000-8000-00000000aa02";
            child_settings_fixture(&home, &root, provider, parent, 2);
            child_settings_fixture(&home, &root, provider, sibling, 0);
            let service = ProjectSyncService::new(&repo);
            let resource_id = ResourceId::parse(format!("{provider}:session:{parent}")).unwrap();
            let before = repo.load_config().unwrap().links[0].clone();
            let review = session_push_review(&service, &project, &storage, &resource_id);
            assert_eq!(review.inventory.resources.len(), 1);
            assert_eq!(review.thread_comparison.as_ref().unwrap().entries.len(), 1);
            assert_eq!(
                review
                    .thread_comparison
                    .as_ref()
                    .unwrap()
                    .capture_budget
                    .as_ref()
                    .unwrap()
                    .reserved_bytes,
                0
            );
            assert!(review.git_history.is_none());
            assert!(review.git_history_review_token.is_none());
            assert!(!review.git_history_changed);
            assert!(review.project_files.is_none());
            let approval = PushApproval::recommended(&review, &storage);
            assert_eq!(
                approval.recipe.entries.keys().collect::<Vec<_>>(),
                [&resource_id]
            );

            let mut invalid = approval.clone();
            let other_id = ResourceId::parse(format!("{provider}:session:{sibling}")).unwrap();
            let mut extra = invalid.recipe.entries[&resource_id].clone();
            extra.resource_id = other_id.clone();
            invalid.recipe.entries.insert(other_id, extra);
            assert!(service
                .push_reviewed(&project.local_project_id, &storage, invalid)
                .unwrap_err()
                .contains("only the selected session"));
            let mut invalid = approval.clone();
            invalid.git_history_review_token = Some("not-approved".into());
            assert!(service
                .push_reviewed(&project.local_project_id, &storage, invalid)
                .unwrap_err()
                .contains("only the selected session"));
            assert_eq!(repo.load_config().unwrap().links[0], before);

            let result = service
                .push_reviewed(&project.local_project_id, &storage, approval.clone())
                .unwrap();
            assert!(result.success, "{result:?}");
            assert_eq!(result.resources_changed, Some(1));
            assert_eq!(result.results.len(), 1);
            assert_eq!(result.results[0].resource_id, resource_id);
            let (_, engine) = storage_engine(&repo, &storage).unwrap();
            let snapshot = engine.inspect(&project.bundle_id).unwrap();
            assert_eq!(snapshot.manifest.all_conversations().count(), 1);
            assert!(snapshot.manifest.git_history.is_none());
            let thread = snapshot.manifest.all_conversations().next().unwrap().2;
            assert_eq!(
                thread.wire_children,
                if children {
                    ConversationWireChildren::On
                } else {
                    ConversationWireChildren::Off
                }
            );
            assert_eq!(
                thread
                    .files
                    .iter()
                    .filter(|file| matches!(
                        file.role,
                        ConversationFileRole::SubagentTranscript { .. }
                    ))
                    .count(),
                if children { 2 } else { 0 }
            );
            let after = repo.load_config().unwrap().links[0].clone();
            assert_eq!(after.recipe, before.recipe);
            assert_eq!(
                after.project_content_preferences,
                before.project_content_preferences
            );
            assert_eq!(
                after
                    .conversation_checkpoints
                    .values()
                    .map(BTreeMap::len)
                    .sum::<usize>(),
                1
            );
            assert_eq!(after.scope_bases.len(), 1);
            let accepted = repo.load_conversation_bindings().unwrap();

            // Reusing a completed review must not publish an empty snapshot
            // or accept the other local session.
            assert!(service
                .push_reviewed(&project.local_project_id, &storage, approval)
                .is_err());
            assert_eq!(
                engine.inspect(&project.bundle_id).unwrap().head,
                snapshot.head
            );
            assert_eq!(repo.load_config().unwrap().links[0], after);
            assert_eq!(repo.load_conversation_bindings().unwrap(), accepted);
            let remaining = service
                .prepare_push_review(&project.local_project_id, &storage)
                .unwrap();
            assert!(
                remaining
                    .thread_comparison
                    .unwrap()
                    .entries
                    .iter()
                    .any(|entry| entry.thread_id.as_str() == sibling
                        && entry.push_decision.selectable)
            );
        }
    }
}

#[test]
fn session_push_preserves_other_stored_sessions_and_provider_resources() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("session-retention").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let project = register(&repo);
    let root = temp.path().join("project");
    let home = temp.path().join("claude");
    let codex_home = temp.path().join("codex");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(home.join("projects/source")).unwrap();
    let claude = ProviderKey::parse("claude").unwrap();
    let profiles = BTreeMap::from([
        (claude.clone(), add_profile(&repo, &claude, &home)),
        (codex_key(), add_profile(&repo, &codex_key(), &codex_home)),
    ]);
    save_project_binding_with_repository(
        &repo,
        SaveProjectBindingRequest {
            local_project_id: project.local_project_id.clone(),
            project_root: root.to_string_lossy().into_owned(),
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
    child_settings_fixture(&home, &root, "claude", parent, 1);
    child_settings_fixture(
        &home,
        &root,
        "claude",
        "019fb002-0000-7000-8000-00000000aa02",
        0,
    );
    let bucket = home.join("projects").join(
        mallard_infrastructure::providers::claude::encode_claude_project_path(
            &root.to_string_lossy(),
        ),
    );
    std::fs::rename(home.join("projects/source"), &bucket).unwrap();
    let memory = bucket.join("memory/notes.md");
    std::fs::create_dir_all(memory.parent().unwrap()).unwrap();
    std::fs::write(&memory, "Stored memory\n").unwrap();
    child_settings_fixture(&codex_home, &root, "codex", parent, 0);
    std::fs::write(root.join("keep.txt"), "Stored project file\n").unwrap();
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&project.local_project_id, &storage)
        .unwrap();
    let files =
        inspect_project_files_with_repository(&repo, &project.local_project_id, &storage).unwrap();
    let mut approval = PushApproval::recommended(&review, &storage);
    let memory_resource = review
        .inventory
        .resources
        .iter()
        .find(|resource| resource.descriptor.resource_id.as_str() == "claude:memory:root")
        .unwrap();
    approval.recipe.entries.insert(
        memory_resource.descriptor.resource_id.clone(),
        RecipeEntry {
            resource_id: memory_resource.descriptor.resource_id.clone(),
            apply_policy: memory_resource.descriptor.apply_policy.clone(),
            required: false,
        },
    );
    approval
        .recipe
        .entries
        .extend(reviewed_project_content_recipe(&files).entries);
    approval.project_content_review_token = files.review_token;
    assert!(
        service
            .push_reviewed(&project.local_project_id, &storage, approval)
            .unwrap()
            .success
    );
    let (_, engine) = storage_engine(&repo, &storage).unwrap();
    let before = engine.inspect(&project.bundle_id).unwrap();
    let link_before = repo.load_config().unwrap().links[0].clone();
    let scope_key = SyncScopeKey::Provider(ProviderKey::parse("claude").unwrap());
    let scope_before = before.manifest.scope(&scope_key).unwrap();
    assert!(
        scope_before
            .files
            .keys()
            .any(|path| path.as_str() == "state/providers/claude/memory/notes.md"),
        "fixture must contain generic provider data"
    );
    std::fs::write(memory, "Local memory must stay local\n").unwrap();
    std::fs::write(root.join("keep.txt"), "Local project change\n").unwrap();
    save_child_push(&repo, "claude", false);
    let resource_id = ResourceId::parse(format!("claude:session:{parent}")).unwrap();
    let review = session_push_review(&service, &project, &storage, &resource_id);
    let row = &review.thread_comparison.as_ref().unwrap().entries[0];
    assert_eq!(row.state, ThreadSyncState::Synced);
    assert!(
        row.push_decision.selectable,
        "an export setting change is publishable"
    );
    let result = service
        .push_reviewed(
            &project.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(result.success, "{result:?}");
    let after = engine.inspect(&project.bundle_id).unwrap();
    let scope_after = after.manifest.scope(&scope_key).unwrap();
    for key in [SyncScopeKey::Project, SyncScopeKey::Provider(codex_key())] {
        assert!(before.manifest.scope(&key).is_some());
        assert_eq!(after.manifest.scope(&key), before.manifest.scope(&key));
        assert_eq!(
            repo.load_config().unwrap().links[0].scope_bases.get(&key),
            link_before.scope_bases.get(&key)
        );
    }
    let changed_resource = &scope_before
        .conversations
        .values()
        .find(|thread| thread.source_thread_id.as_deref() == Some(parent))
        .unwrap()
        .resource_id;
    assert_eq!(
        scope_after
            .files
            .iter()
            .filter(|(_, file)| &file.resource_id != changed_resource)
            .collect::<Vec<_>>(),
        scope_before
            .files
            .iter()
            .filter(|(_, file)| &file.resource_id != changed_resource)
            .collect::<Vec<_>>(),
    );
    assert_eq!(scope_after.directories, scope_before.directories);
    let changed = scope_after
        .conversations
        .iter()
        .filter(|(id, thread)| scope_before.conversations.get(id) != Some(thread))
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    assert_eq!(changed.len(), 1);
    let link_after = repo.load_config().unwrap().links[0].clone();
    assert_eq!(link_after.recipe, link_before.recipe);
    assert_eq!(
        link_after.project_content_preferences,
        link_before.project_content_preferences
    );
    let checkpoints_before = &link_before.conversation_checkpoints[&scope_key];
    let checkpoints_after = &link_after.conversation_checkpoints[&scope_key];
    assert_eq!(
        checkpoints_after
            .iter()
            .filter(|(id, checkpoint)| checkpoints_before.get(id) != Some(checkpoint))
            .count(),
        1
    );
    after.manifest.validate().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_push_preserves_stored_git_commits_and_maps_the_final_sessions() {
    managed_cloud_gone::with_cloud(|cloud| {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage = managed_cloud_gone::add_cloud_storage(
            &repo,
            &cloud,
            "session-owner",
            "session-history",
        );
        let (project, root) = git_history_project(&repo, temp.path(), &storage);
        let id = "019f7500-0000-7000-8000-000000000006";
        let path = temp
            .path()
            .join(format!("codex-home/sessions/2026/07/22/rollout-{id}.jsonl"));
        let held = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let service = ProjectSyncService::new(&repo);
        push_child_settings(&service, &project, &storage);
        let (_, engine) = storage_engine(&repo, &storage).unwrap();
        let before = engine.inspect(&project.bundle_id).unwrap();
        assert_eq!(before.manifest.all_conversations().count(), 6);
        let stored_history = before.manifest.git_history.as_ref().unwrap();
        let link_before = repo.load_config().unwrap().links[0].clone();
        std::fs::write(path, held).unwrap();
        let resource_id = ResourceId::parse(format!("codex:session:{id}")).unwrap();
        let review = session_push_review(&service, &project, &storage, &resource_id);
        assert!(review.git_history.is_none());
        assert!(review.git_history_review_token.is_none());
        // A branch switch and a new local commit after review do not add Git
        // work to a session publication.
        git_history_test_command(&root, &["checkout", "-b", "session-local"], None);
        git_history_test_command(
            &root,
            &["commit", "--allow-empty", "-m", "Unrelated local commit"],
            Some("2026-07-22T00:05:00Z"),
        );
        let result = service
            .push_reviewed(
                &project.local_project_id,
                &storage,
                PushApproval::recommended(&review, &storage),
            )
            .unwrap();
        assert!(result.success, "{result:?}");
        let after = engine.inspect(&project.bundle_id).unwrap();
        assert_eq!(after.manifest.all_conversations().count(), 7);
        let mut expected_history = stored_history.clone();
        expected_history.map_sessions(
            after
                .manifest
                .all_conversations()
                .map(|(_, _, thread)| thread),
        );
        assert_eq!(after.manifest.git_history.as_ref(), Some(&expected_history));
        assert_ne!(
            expected_history, *stored_history,
            "the new session needs a reference"
        );
        assert_eq!(
            repo.load_config().unwrap().links[0].recipe,
            link_before.recipe
        );
        after.manifest.validate().unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_push_to_empty_cloud_does_not_create_git_history() {
    managed_cloud_gone::with_cloud(|cloud| {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage = managed_cloud_gone::add_cloud_storage(
            &repo,
            &cloud,
            "session-owner",
            "session-no-history",
        );
        let (project, _) = git_history_project(&repo, temp.path(), &storage);
        let service = ProjectSyncService::new(&repo);
        let resource_id =
            ResourceId::parse("codex:session:019f7500-0000-7000-8000-000000000000").unwrap();
        let review = session_push_review(&service, &project, &storage, &resource_id);
        let result = service
            .push_reviewed(
                &project.local_project_id,
                &storage,
                PushApproval::recommended(&review, &storage),
            )
            .unwrap();
        assert!(result.success, "{result:?}");
        let snapshot = storage_engine(&repo, &storage)
            .unwrap()
            .1
            .inspect(&project.bundle_id)
            .unwrap();
        assert_eq!(snapshot.manifest.all_conversations().count(), 1);
        assert!(snapshot.manifest.git_history.is_none());
        snapshot.manifest.validate().unwrap();
    })
    .await;
}

#[test]
#[ignore = "Prepare an isolated session Push fixture for native UI checks"]
fn session_push_native_ui_fixture() {
    let root =
        std::fs::canonicalize(std::env::var("MALLARD_SESSION_PUSH_QA_ROOT").unwrap()).unwrap();
    assert!(root.starts_with("/private/tmp"));
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("mallard-session-push-qa-"));
    assert!(!root.join("home/.mallard").exists());
    let repo = LocalStateRepository::from_home_dir(root.join("home")).unwrap();
    let storage = StorageId::parse("session-push-qa").unwrap();
    add_local_storage(&repo, &storage, &root.join("store"));
    let project = register(&repo);
    let project_root = root.join("project");
    let home = root.join("claude");
    bind_and_link_claude_replica(&repo, &project, &storage, &project_root, &home);
    for (id, title) in [
        (
            "11111111-1111-4111-8111-111111111111",
            "Push only this session",
        ),
        (
            "22222222-2222-4222-8222-222222222222",
            "Keep this session local",
        ),
    ] {
        let bytes = claude_test_transcript(
            id,
            &project_root,
            &[
                ("u1", None, "user", title),
                (
                    "a1",
                    Some("u1"),
                    "assistant",
                    "Ready for the isolated Push check.",
                ),
            ],
        );
        let mut current = Vec::new();
        for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
            let mut record: serde_json::Value = serde_json::from_slice(line).unwrap();
            record["timestamp"] = serde_json::json!("2026-10-02T20:00:00Z");
            serde_json::to_writer(&mut current, &record).unwrap();
            current.push(b'\n');
        }
        std::fs::write(home.join(format!("projects/source/{id}.jsonl")), current).unwrap();
    }
}
