// Included in the service tests so all state and provider homes use the
// existing temporary fixture. No real Mallard or agent home is read.

fn ensure_test_git_repository(project: &LocalProjectRegistration, root: &Path) {
    if project.repository_fingerprint.is_some() {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .unwrap();
        if !output.status.success() {
            git_history_test_command(root, &["init", "-b", "main"], None);
        }
    }
}

fn git_history_test_command(root: &Path, args: &[&str], time: Option<&str>) {
    let mut command = std::process::Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    if let Some(time) = time {
        command
            .env("GIT_AUTHOR_DATE", time)
            .env("GIT_COMMITTER_DATE", time);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A Git project with three commits, linked to `storage_id`, and seven Codex
/// sessions: two per successor commit and one after the last.
fn git_history_project(
    repo: &LocalStateRepository,
    temp: &Path,
    storage_id: &StorageId,
) -> (LocalProjectRegistration, PathBuf) {
    let root = temp.join("project");
    fs::create_dir_all(&root).unwrap();
    git_history_test_command(&root, &["init", "-b", "main"], None);
    git_history_test_command(&root, &["config", "user.name", "Mallard Test"], None);
    git_history_test_command(
        &root,
        &["config", "user.email", "mallard@example.test"],
        None,
    );
    git_history_test_command(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/history",
        ],
        None,
    );
    for (subject, time) in [
        ("Create project", "2026-07-22T00:00:00Z"),
        ("Add story", "2026-07-22T00:01:00Z"),
        ("Add artwork", "2026-07-22T00:02:00Z"),
    ] {
        git_history_test_command(
            &root,
            &["commit", "--allow-empty", "-m", subject],
            Some(time),
        );
    }
    let project = register_local_project_with_repository(
        repo,
        RegisterLocalProjectRequest {
            display_name: "Git history".into(),
            repository_fingerprint: repository_fingerprint(&root),
            bundle_id: None,
        },
    )
    .unwrap();
    let codex_home = temp.join("codex-home");
    let profile = add_profile(repo, &codex_key(), &codex_home);
    save_project_binding_with_repository(
        repo,
        SaveProjectBindingRequest {
            local_project_id: project.local_project_id.clone(),
            project_root: root.to_string_lossy().into_owned(),
            profile_ids: BTreeMap::from([(codex_key(), profile)]),
            expected_revision: None,
        },
    )
    .unwrap();
    save_project_link_with_repository(
        repo,
        SaveProjectLinkRequest {
            local_project_id: project.local_project_id.clone(),
            storage_id: storage_id.clone(),
            pinned: true,
        },
    )
    .unwrap();

    let sessions = codex_home.join("sessions/2026/07/22");
    fs::create_dir_all(&sessions).unwrap();
    for index in 0..7 {
        let id = format!("019f7500-0000-7000-8000-{index:012}");
        let transcript = command_test_transcript(
            &id,
            &root,
            &[
                ("prompt", "user", "Make artwork"),
                ("reply", "assistant", "Done"),
            ],
        );
        let mut timed = Vec::new();
        for raw in transcript
            .split(|byte| *byte == b'\n')
            .filter(|raw| !raw.is_empty())
        {
            let mut record: serde_json::Value = serde_json::from_slice(raw).unwrap();
            // Two sessions per successor commit, one uncommitted. End time is
            // later than the last prompt; mapping must use the prompt.
            let prompt_time = format!("2026-07-22T00:0{}:00Z", index / 2);
            record["timestamp"] =
                if record["type"] == "session_meta" || record["payload"]["role"] == "user" {
                    serde_json::json!(prompt_time)
                } else {
                    serde_json::json!("2026-07-22T00:04:00Z")
                };
            serde_json::to_writer(&mut timed, &record).unwrap();
            timed.push(b'\n');
        }
        // Current Codex records counted user input separately from the model
        // input response_item. Include the native event pair as a real rollout does.
        for event in [
            serde_json::json!({ "ordinal": 3, "timestamp": format!("2026-07-22T00:0{}:00Z", index / 2),
                "type": "event_msg", "payload": { "type": "user_message", "message": "Make artwork" } }),
            serde_json::json!({ "ordinal": 4, "timestamp": "2026-07-22T00:04:00Z", "type": "event_msg",
                "payload": { "type": "agent_message", "phase": "final_answer", "message": "Done" } }),
        ] {
            serde_json::to_writer(&mut timed, &event).unwrap();
            timed.push(b'\n');
        }
        fs::write(sessions.join(format!("rollout-{id}.jsonl")), timed).unwrap();
    }
    (project, root)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_history_push_to_mallard_cloud_maps_final_sessions_and_publishes_git_only_changes() {
    managed_cloud_gone::with_cloud(|cloud| {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = managed_cloud_gone::add_cloud_storage(
            &repo,
            &cloud,
            "history-owner",
            "git-history-cloud",
        );
        let (project, root) = git_history_project(&repo, temp.path(), &storage_id);
        let review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert!(review.git_history_changed);
        assert_eq!(review.git_history.as_ref().unwrap().commits.len(), 3);
        // Storage holds no history yet, so nothing reads as new.
        assert_eq!(review.git_history_outgoing, None);
        let result = ProjectSyncService::new(&repo)
            .push_reviewed(
                &project.local_project_id,
                &storage_id,
                PushApproval::recommended(&review, &storage_id),
            )
            .unwrap();
        assert!(result.success, "{}", result.message);
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let first = engine.inspect(&project.bundle_id).unwrap();
        let history = first.manifest.git_history.as_ref().unwrap();
        assert_eq!(history.branch, "main");
        assert_eq!(
            history
                .commits
                .iter()
                .map(|commit| commit.session_refs.len())
                .collect::<Vec<_>>(),
            [2, 2, 2]
        );
        assert_eq!(history.uncommitted_session_refs.len(), 1);
        assert_eq!(first.manifest.all_conversations().count(), 7);
        first.manifest.validate().unwrap();
        let mut fallback_sessions: Vec<_> = first
            .manifest
            .all_conversations()
            .map(|(_, _, thread)| thread.clone())
            .collect();
        let fallback_id = fallback_sessions[0].resource_id.clone();
        fallback_sessions[0].presentation.last_prompt_at = None;
        fallback_sessions[0].presentation.ended_at = Some(history.commits[1].committed_at);
        let mut fallback = history.clone();
        fallback.map_sessions(fallback_sessions.iter());
        assert!(fallback.commits[1].session_refs.contains(&fallback_id));
        fallback_sessions[0].presentation.ended_at = None;
        fallback_sessions[0].updated_at = history.commits[0].committed_at;
        fallback.map_sessions(fallback_sessions.iter());
        assert!(fallback.commits[0].session_refs.contains(&fallback_id));

        let unchanged =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert!(!unchanged.git_history_changed);
        assert_eq!(
            unchanged.git_history_outgoing,
            Some(mallard_domain::GitHistoryOutgoing {
                new_commits: 0,
                rewritten: false
            })
        );
        let no_change = ProjectSyncService::new(&repo)
            .push_reviewed(
                &project.local_project_id,
                &storage_id,
                PushApproval::recommended(&unchanged, &storage_id),
            )
            .unwrap();
        assert!(no_change.success, "{}", no_change.message);
        assert_eq!(engine.inspect(&project.bundle_id).unwrap().head, first.head);
        git_history_test_command(
            &root,
            &["commit", "--allow-empty", "-m", "Save uncommitted work"],
            Some("2026-07-22T00:05:00Z"),
        );
        let stale = ProjectSyncService::new(&repo)
            .push_reviewed(
                &project.local_project_id,
                &storage_id,
                PushApproval::recommended(&unchanged, &storage_id),
            )
            .unwrap_err();
        assert!(
            stale.contains("Git history changed after review"),
            "{stale}"
        );
        assert_eq!(engine.inspect(&project.bundle_id).unwrap().head, first.head);

        let changed =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert!(changed.git_history_changed);
        assert_eq!(
            changed.git_history_outgoing,
            Some(mallard_domain::GitHistoryOutgoing {
                new_commits: 1,
                rewritten: false
            })
        );
        let mut approval = PushApproval::recommended(&changed, &storage_id);
        approval.recipe.entries.clear();
        let result = ProjectSyncService::new(&repo)
            .push_reviewed(&project.local_project_id, &storage_id, approval)
            .unwrap();
        assert!(result.success, "{}", result.message);
        let second = engine.inspect(&project.bundle_id).unwrap();
        assert_eq!(second.head.revision, first.head.revision + 1);
        assert_ne!(second.head.manifest_sha256, first.head.manifest_sha256);
        assert_eq!(second.manifest.scopes, first.manifest.scopes);
        assert_eq!(
            second.manifest.git_history.as_ref().unwrap().commits.len(),
            4
        );
        assert!(second
            .manifest
            .git_history
            .as_ref()
            .unwrap()
            .uncommitted_session_refs
            .is_empty());
        assert_eq!(
            engine
                .inspect_manifest_version(
                    &project.bundle_id,
                    first.head.revision,
                    &first.head.commit_id,
                    first.head.manifest_size,
                    &first.head.manifest_sha256
                )
                .unwrap(),
            first.manifest
        );
        fs::remove_dir_all(root.join(".git")).unwrap();
        let missing =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap_err();
        assert!(missing.contains("Git project is unavailable"), "{missing}");
        assert_eq!(
            engine.inspect(&project.bundle_id).unwrap().head,
            second.head
        );
    })
    .await;
}

#[test]
fn git_history_stays_off_storages_that_do_not_show_it() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage_id = StorageId::parse("git-history-store").unwrap();
    add_local_storage(&repo, &storage_id, &temp.path().join("store"));
    let (project, root) = git_history_project(&repo, temp.path(), &storage_id);

    let review =
        prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
    assert_eq!(review.git_history, None);
    assert_eq!(review.git_history_review_token, None);
    assert!(!review.git_history_changed);
    assert_eq!(review.git_history_outgoing, None);
    let result = ProjectSyncService::new(&repo)
        .push_reviewed(
            &project.local_project_id,
            &storage_id,
            PushApproval::recommended(&review, &storage_id),
        )
        .unwrap();
    assert!(result.success, "{}", result.message);
    let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
    let first = engine.inspect(&project.bundle_id).unwrap();
    assert_eq!(first.manifest.all_conversations().count(), 7);
    assert_eq!(first.manifest.git_history, None);

    // A new commit is no change here, so a Push of nothing publishes nothing.
    git_history_test_command(
        &root,
        &["commit", "--allow-empty", "-m", "Save more work"],
        Some("2026-07-22T00:05:00Z"),
    );
    let after =
        prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
    assert!(!after.git_history_changed);
    let mut approval = PushApproval::recommended(&after, &storage_id);
    approval.recipe.entries.clear();
    ProjectSyncService::new(&repo)
        .push_reviewed(&project.local_project_id, &storage_id, approval)
        .unwrap();
    assert_eq!(engine.inspect(&project.bundle_id).unwrap().head, first.head);

    // A missing repository does not block a storage that never reads it.
    fs::remove_dir_all(root.join(".git")).unwrap();
    prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
}
