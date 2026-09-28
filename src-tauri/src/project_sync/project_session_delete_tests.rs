// Bulk deletion uses the same provider operations as a session-row deletion.
#[test]
#[cfg(unix)]
fn project_sessions_delete_preserves_other_projects_and_reports_partial_failures() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for failure in ["none", "provider", "unavailable", "metadata"] {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage = StorageId::parse("delete-store").unwrap();
        add_local_storage(&repo, &storage, &temp.path().join("store"));
        let selected = register(&repo);
        let root = temp.path().join("project");
        let home = temp.path().join("claude");
        bind_and_link_claude_replica(&repo, &selected, &storage, &root, &home);
        let (first, children) = write_claude_owned_fixture(&home, &root, "selected-first", 2);
        let (second, _) = write_claude_owned_fixture(&home, &root, "selected-second", 1);
        let project_file = root.join("keep.txt");
        std::fs::write(&project_file, "project content").unwrap();
        let other = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Other project".into(),
                repository_fingerprint: None,
                bundle_id: Some(BundleId::parse("1123456789abcdef0123456789abcdef").unwrap()),
            },
        )
        .unwrap();
        let other_root = root.join("nested-project");
        std::fs::create_dir_all(&other_root).unwrap();
        let profile_ids = repo
            .load_bindings()
            .unwrap()
            .active_for(&selected.local_project_id)
            .unwrap()
            .profile_ids
            .clone();
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: other.local_project_id.clone(),
                project_root: other_root.to_string_lossy().into_owned(),
                profile_ids,
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: other.local_project_id.clone(),
                storage_id: storage.clone(),
                pinned: true,
            },
        )
        .unwrap();
        let (foreign, foreign_children) =
            write_claude_owned_fixture(&home, &other_root, "other-project", 1);
        let unbound_home = temp.path().join("unbound-claude");
        let (unbound, _) = write_claude_owned_fixture(&unbound_home, &root, "unbound-profile", 1);
        let service = ProjectSyncService::new(&repo);
        for project in [&selected, &other] {
            let review = service
                .prepare_push_review(&project.local_project_id, &storage)
                .unwrap();
            let pushed = service
                .push_reviewed(
                    &project.local_project_id,
                    &storage,
                    PushApproval::recommended(&review, &storage),
                )
                .unwrap();
            assert!(pushed.success, "{}", pushed.message);
        }
        let stored_bytes = || {
            walkdir::WalkDir::new(temp.path().join("store"))
                .into_iter()
                .map(Result::unwrap)
                .filter(|entry| entry.file_type().is_file())
                .map(|entry| {
                    (
                        entry.path().to_path_buf(),
                        std::fs::read(entry.path()).unwrap(),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        };
        let cloud_before = stored_bytes();
        let config_before = repo.load_config().unwrap();
        let bindings_before = repo.load_conversation_bindings().unwrap();
        let materials_before = repo.load_materializations().unwrap();
        let revision = config_before
            .project(&selected.local_project_id)
            .unwrap()
            .revision;
        assert!(delete_local_project_sessions_with_repository(
            &repo,
            &selected.local_project_id,
            revision + 1
        )
        .is_err());
        assert!(
            first.exists() && second.exists(),
            "stale request must not delete files"
        );
        assert_eq!(repo.load_conversation_bindings().unwrap(), bindings_before);
        let moved = temp.path().join("moved");
        if failure == "provider" {
            let child_dir = first.with_extension("");
            std::fs::rename(&child_dir, &moved).unwrap();
            symlink(&moved, &child_dir).unwrap();
        } else if failure == "unavailable" {
            std::fs::rename(&home, &moved).unwrap();
        }
        let permissions = std::fs::metadata(repo.root()).unwrap().permissions();
        if failure == "metadata" {
            std::fs::set_permissions(repo.root(), std::fs::Permissions::from_mode(0o500)).unwrap();
        }
        let result = delete_local_project_sessions_with_repository(
            &repo,
            &selected.local_project_id,
            revision,
        )
        .unwrap();
        std::fs::set_permissions(repo.root(), permissions).unwrap();
        if failure == "unavailable" {
            std::fs::rename(&moved, &home).unwrap();
        }
        assert_eq!(
            result.sessions_deleted,
            match failure {
                "provider" => 1,
                "unavailable" => 0,
                _ => 2,
            }
        );
        assert_eq!(result.sessions_failed, u64::from(failure == "provider"));
        assert_eq!(
            !result.provider_errors.is_empty(),
            matches!(failure, "provider" | "unavailable")
        );
        assert_eq!(result.metadata_error.is_some(), failure == "metadata");
        assert_eq!(
            first.exists(),
            matches!(failure, "provider" | "unavailable")
        );
        assert_eq!(second.exists(), failure == "unavailable");
        assert!(foreign.exists() && foreign_children[0].exists() && unbound.exists());
        assert_eq!(
            std::fs::read_to_string(&project_file).unwrap(),
            "project content"
        );
        if failure == "none" {
            assert!(children.iter().all(|path| !path.exists()));
        }
        let after = repo.load_conversation_bindings().unwrap();
        let expected_bindings: Vec<_> = bindings_before
            .bindings
            .into_iter()
            .filter(|binding| {
                failure == "metadata" || binding.local_project_id != selected.local_project_id
            })
            .collect();
        assert_eq!(after.bindings, expected_bindings);
        let after_config = repo.load_config().unwrap();
        for (before, after) in config_before.links.iter().zip(&after_config.links) {
            if before.local_project_id == selected.local_project_id && failure != "metadata" {
                assert!(after.scope_bases.is_empty() && after.conversation_checkpoints.is_empty());
                assert_eq!(before.recipe, after.recipe);
            } else {
                assert_eq!(before, after);
            }
        }
        let expected_materials: Vec<_> = materials_before
            .records
            .into_iter()
            .filter(|record| {
                failure == "metadata" || record.local_project_id != selected.local_project_id
            })
            .collect();
        assert_eq!(
            repo.load_materializations().unwrap().records,
            expected_materials
        );
        assert_eq!(config_before.projects, after_config.projects);
        // Existing cloud content remains readable after local deletion.
        assert_eq!(stored_bytes(), cloud_before);
    }
}

#[test]
#[ignore = "requires an installed Codex CLI; temporary homes only"]
fn project_sessions_delete_mixed_native_includes_archived_and_unbound_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("mixed-delete-store").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let project = register(&repo);
    let root = temp.path().join("project");
    let claude_home = temp.path().join("claude");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(claude_home.join("projects/source")).unwrap();
    let claude = ProviderKey::parse("claude").unwrap();
    let claude_profile = add_profile(&repo, &claude, &claude_home);
    let (claude_root, claude_children) =
        write_claude_owned_fixture(&claude_home, &root, "selected-claude", 2);
    let codex_home = temp.path().join("codex");
    let codex_profile = add_profile(&repo, &codex_key(), &codex_home);
    save_project_binding_with_repository(
        &repo,
        SaveProjectBindingRequest {
            local_project_id: project.local_project_id.clone(),
            project_root: root.to_string_lossy().into_owned(),
            profile_ids: BTreeMap::from([(claude, claude_profile), (codex_key(), codex_profile)]),
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
    let foreign_root = temp.path().join("other-project");
    std::fs::create_dir_all(&foreign_root).unwrap();
    let ids = [
        "019f7798-5437-7632-9dbc-5b589cf68bf0",
        "019f7798-5437-7632-9dbc-5b589cf68bf1",
        "019f7798-5437-7632-9dbc-5b589cf68bf2",
        "019f7798-5437-7632-9dbc-5b589cf68bf3",
    ];
    let mut paths = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let bucket = if index == 1 {
            "archived_sessions"
        } else {
            "sessions/2026/09/26"
        };
        let dir = codex_home.join(bucket);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-2026-09-26T00-00-00-{id}.jsonl"));
        let mut payload = serde_json::json!({"id":id,"cwd":if index == 3 { &foreign_root } else { &root },
            "timestamp":"2026-09-26T00:00:00Z","source":"cli","cli_version":"0.156.0","originator":"codex_cli_rs"});
        if index == 0 {
            payload["forked_from_id"] = serde_json::json!(ids[3]);
        }
        if index == 2 {
            payload["source"] = serde_json::json!({"subagent":{"thread_spawn":{"parent_thread_id":ids[0],"depth":1}}});
            payload["parent_thread_id"] = serde_json::json!(ids[0]);
        }
        std::fs::write(&path, format!("{}\n", serde_json::json!({"type":"session_meta","timestamp":"2026-09-26T00:00:00Z","payload":payload}))).unwrap();
        paths.push(path);
    }
    assert!(repo
        .load_conversation_bindings()
        .unwrap()
        .bindings
        .is_empty());
    let revision = repo
        .load_config()
        .unwrap()
        .project(&project.local_project_id)
        .unwrap()
        .revision;
    let result =
        delete_local_project_sessions_with_repository(&repo, &project.local_project_id, revision)
            .unwrap();
    assert_eq!(result.sessions_deleted, 3, "{result:?}");
    assert!(
        result.provider_errors.is_empty() && result.metadata_error.is_none(),
        "{result:?}"
    );
    assert!(!claude_root.exists() && claude_children.iter().all(|path| !path.exists()));
    assert!(paths[..3].iter().all(|path| !path.exists()));
    assert!(paths[3].exists(), "foreign parent must remain");
}
