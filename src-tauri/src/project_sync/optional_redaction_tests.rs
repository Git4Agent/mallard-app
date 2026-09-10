// Synthetic fixtures only. Included in service::tests.
fn save_test_redaction(repo: &LocalStateRepository, enabled: bool) {
    let mut settings = mallard_infrastructure::app_settings::load_app_settings(repo).unwrap();
    settings.credential_redaction_enabled = enabled;
    mallard_infrastructure::app_settings::save_app_settings(repo, &settings).unwrap();
}

#[test]
fn optional_redaction_session_children_pull_adoption_and_republication() {
    const SECRET: &str = "sk-ant-api03-aaaabbbbccccddddeeee";
    for provider in ["codex", "claude"] {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage = StorageId::parse("optional-redaction-store").unwrap();
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
        let parent = "019fb002-0000-7000-8000-00000000aa01";
        let paths = if provider == "codex" {
            let child = "019fb002-0000-7000-8000-00000000bb02";
            vec![
                write_owned_rollout(&home, &root, parent, None, &[("start-child", child)]),
                write_owned_rollout(&home, &root, child, Some(parent), &[]),
            ]
        } else {
            let (main, children) = write_claude_owned_fixture(&home, &root, parent, 1);
            let old_bucket = home.join("projects/source");
            let bucket = home.join("projects").join(
                mallard_infrastructure::providers::claude::encode_claude_project_path(
                    root.to_str().unwrap(),
                ),
            );
            std::fs::rename(&old_bucket, &bucket).unwrap();
            std::iter::once(main)
                .chain(children)
                .map(|path| bucket.join(path.strip_prefix(&old_bucket).unwrap()))
                .collect::<Vec<_>>()
        };
        for path in &paths {
            let text = std::fs::read_to_string(path)
                .unwrap()
                .replace("test owned child", SECRET)
                .replace("launch agents", SECRET)
                .replace("child task", SECRET);
            std::fs::write(path, text).unwrap();
        }
        if provider == "claude" {
            let history = home.join("file-history").join(parent);
            std::fs::create_dir_all(&history).unwrap();
            std::fs::write(history.join("env@v1"), format!("access_token={SECRET}\n")).unwrap();
            let memory = paths[0].parent().unwrap().join("memory");
            std::fs::create_dir_all(&memory).unwrap();
            std::fs::write(memory.join("notes.md"), format!("access_token={SECRET}\n")).unwrap();
        }
        if provider == "codex" {
            std::fs::write(
                home.join("session_index.jsonl"),
                format!(
                    "{}\n",
                    serde_json::json!({"id": parent, "thread_name": SECRET})
                ),
            )
            .unwrap();
        }
        let service = ProjectSyncService::new(&repo);
        let push = |review: &PushReviewContext| {
            let mut approval = PushApproval::recommended(review, &storage);
            for resource in &review.inventory.resources {
                if resource.descriptor.kind == contracts::provider_state() {
                    let id = resource.descriptor.resource_id.clone();
                    approval.recipe.entries.insert(
                        id.clone(),
                        RecipeEntry {
                            resource_id: id,
                            apply_policy: contracts::merge(),
                            required: false,
                        },
                    );
                }
            }
            let result = service
                .push_reviewed(&source.local_project_id, &storage, approval)
                .unwrap();
            assert!(
                result.success,
                "{provider}: {} {:?}",
                result.message, result.results
            );
        };
        let inspect = || {
            storage_engine(&repo, &storage)
                .unwrap()
                .1
                .inspect(&source.bundle_id)
                .unwrap()
        };
        let verify = |snapshot: &BundleSnapshot, enabled: bool| {
            let (_, engine) = storage_engine(&repo, &storage).unwrap();
            let (_, _, thread) = snapshot.manifest.all_conversations().next().unwrap();
            assert_eq!(
                thread.wire_redaction.as_ref().unwrap(),
                &credential_redaction::shared()
                    .begin_run()
                    .with_enabled(enabled)
                    .stamp()
            );
            let mut bytes = engine.byte_source(snapshot, None);
            let mut planted_files = 0;
            for (path, _) in snapshot.manifest.all_files() {
                let body = bytes
                    .read_bounded(path, 1024 * 1024, "test payload")
                    .unwrap();
                let text = String::from_utf8_lossy(&body);
                assert_eq!(
                    text.contains(SECRET),
                    !enabled,
                    "{provider}: {path}: {text}"
                );
                planted_files += 1;
            }
            assert!(planted_files >= 2, "parent and child must ship");
            if provider == "claude" {
                assert!(planted_files >= 4, "sidecar and provider state must ship");
            }
            let manifest = serde_json::to_string(&snapshot.manifest).unwrap();
            assert_eq!(
                manifest.contains(SECRET),
                !enabled,
                "{provider}: content title follows mode: {:?}",
                thread.display_title
            );
        };
        let review = service
            .prepare_push_review(&source.local_project_id, &storage)
            .unwrap();
        assert!(review.credential_redaction_enabled);
        push(&review);
        let on = inspect();
        verify(&on, true);
        let cloud_id = on.manifest.all_conversations().next().unwrap().1.clone();

        save_test_redaction(&repo, false);
        let review = service
            .prepare_push_review(&source.local_project_id, &storage)
            .unwrap();
        assert!(!review.credential_redaction_enabled);
        let entries = &review.thread_comparison.as_ref().unwrap().entries;
        assert_eq!(
            entries[0].state,
            ThreadSyncState::Synced,
            "{provider}: {entries:?}"
        );
        assert_eq!(
            entries[0].push_decision.action,
            ConversationReviewAction::Publish
        );
        assert_eq!(
            entries[0].status_detail.as_deref(),
            Some("Export settings changed")
        );
        save_test_redaction(&repo, true);
        let stale = service
            .push_reviewed(
                &source.local_project_id,
                &storage,
                PushApproval::recommended(&review, &storage),
            )
            .unwrap_err();
        assert!(stale.contains("redaction changed after review"), "{stale}");
        assert_eq!(inspect().head, on.head);
        save_test_redaction(&repo, false);
        push(&review);
        let off = inspect();
        verify(&off, false);
        assert!(off.head.revision > on.head.revision);
        assert_eq!(
            *off.manifest.all_conversations().next().unwrap().1,
            cloud_id
        );
        let current = service
            .prepare_push_review(&source.local_project_id, &storage)
            .unwrap();
        assert_eq!(
            current.thread_comparison.as_ref().unwrap().entries[0]
                .push_decision
                .action,
            ConversationReviewAction::NoChanges
        );

        // First-contact adoption must compare against the stored Off stamp, even with local On.
        save_test_redaction(&repo, true);
        repo.mutate_config(|config| {
            let link = config
                .links
                .iter_mut()
                .find(|link| {
                    link.local_project_id == source.local_project_id && link.storage_id == storage
                })
                .unwrap();
            for scope in link.conversation_checkpoints.values_mut() {
                for checkpoint in scope.values_mut() {
                    checkpoint.accepted = None;
                    checkpoint.accepted_local_tail = None;
                    checkpoint.accepted_subagents.clear();
                }
            }
            Ok(())
        })
        .unwrap();
        let adopted = service
            .prepare_push_review(&source.local_project_id, &storage)
            .unwrap();
        let entries = &adopted.thread_comparison.as_ref().unwrap().entries;
        assert_eq!(
            entries[0].state,
            ThreadSyncState::Synced,
            "{provider}: {entries:?}"
        );
        assert_eq!(
            entries[0].push_decision.action,
            ConversationReviewAction::Publish
        );

        let target = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "opposite-mode-target".into(),
                repository_fingerprint: source.repository_fingerprint.clone(),
                bundle_id: Some(source.bundle_id.clone()),
            },
        )
        .unwrap();
        let target_home = temp.path().join("target-provider");
        bind(&target, &temp.path().join("target"), &target_home);
        let bindings = repo.load_bindings().unwrap();
        let binding = bindings.active_for(&target.local_project_id).unwrap();
        let plan = service
            .plan_bundle_restore(&storage, &source.bundle_id, binding)
            .unwrap();
        assert!(plan
            .conversation_review
            .iter()
            .all(|row| row.stored_without_credential_redaction));
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
            "{provider}: {} {:?}",
            result.message, result.failed_actions
        );
        let mut installed = Vec::new();
        jsonl_files_under(&target_home, &mut installed);
        let preserved = installed
            .iter()
            .filter(|path| std::fs::read_to_string(path).unwrap().contains(SECRET))
            .count();
        assert!(
            preserved >= 2,
            "{provider}: parent and child must preserve Off content"
        );

        let review = service
            .prepare_push_review(&source.local_project_id, &storage)
            .unwrap();
        push(&review);
        let on_again = inspect();
        verify(&on_again, true);
        assert_eq!(
            *on_again.manifest.all_conversations().next().unwrap().1,
            cloud_id
        );
        assert!(on_again.head.revision > off.head.revision);
        // The immutable Off revision remains intact.
        verify(&off, false);
        for path in &paths {
            assert!(std::fs::read_to_string(path).unwrap().contains(SECRET));
        }
    }
}
