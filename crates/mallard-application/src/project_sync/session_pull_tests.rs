    // A Pull narrowed to one session (docs/PLAN_DEEP_LINK_SESSION_PULL.md).
    // Synthetic provider homes only. Included in session_refresh_regressions.

    /// The id of the stored row of the session a provider calls `session`:
    /// what a session page's address names.
    fn stored_session_id(
        repo: &LocalStateRepository,
        storage: &StorageId,
        bundle_id: &BundleId,
        session: &str,
    ) -> ResourceId {
        let (_, engine) = storage_engine(repo, storage).unwrap();
        engine
            .inspect(bundle_id)
            .unwrap()
            .manifest
            .all_conversations()
            .find(|(_, _, thread)| thread.source_thread_id.as_deref() == Some(session))
            .map(|(_, _, thread)| thread.resource_id.clone())
            .expect("the session is stored")
    }

    fn session_pull_plan(
        repo: &LocalStateRepository,
        storage: &StorageId,
        project: &LocalProjectRegistration,
        resource_id: &ResourceId,
    ) -> Result<RestorePlan, String> {
        let bindings = repo.load_bindings().unwrap();
        ProjectSyncService::new(repo).plan_bundle_restore_with_progress(
            storage,
            &project.bundle_id,
            bindings.active_for(&project.local_project_id).unwrap(),
            &PullScope::Session {
                resource_id: resource_id.clone(),
            },
            &mut |_| {},
            &mut |_, _| {},
        )
    }

    fn link_of(
        repo: &LocalStateRepository,
        project: &LocalProjectRegistration,
    ) -> ProjectStorageLink {
        repo.load_config()
            .unwrap()
            .links
            .into_iter()
            .find(|link| link.local_project_id == project.local_project_id)
            .expect("the project is linked")
    }

    /// Whether a Push of `project` is refused until its Pull is reviewed.
    fn push_requires_pull(world: &TwoComputers, project: &LocalProjectRegistration) -> bool {
        matches!(
            prepare_push_review_with_repository(
                &world.repo,
                &project.local_project_id,
                &world.storage
            )
            .unwrap()
            .destination_status,
            PushDestinationStatus::RequiresPull { .. }
        )
    }

    fn entries_of_project(
        world: &TwoComputers,
        project: &LocalProjectRegistration,
    ) -> Vec<ThreadSyncEntry> {
        get_project_thread_sync_comparison_with_repository(
            &world.repo,
            &project.local_project_id,
            &world.storage,
        )
        .unwrap()
        .entries
    }

    /// The first computer wrote two sessions and pushed them. Returns the
    /// stored id of `TwoComputers::SESSION`.
    fn two_stored_sessions(provider: &'static str) -> (TwoComputers, ResourceId) {
        let world = TwoComputers::of(provider);
        child_settings_fixture(
            &world.first_home,
            &world.first_root,
            provider,
            TwoComputers::SESSION,
            0,
        );
        child_settings_fixture(
            &world.first_home,
            &world.first_root,
            provider,
            "019fb019-0000-7000-8000-00000000000b",
            0,
        );
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{provider}: {pushed:?}");
        let wanted = stored_session_id(
            &world.repo,
            &world.storage,
            &world.bundle_id,
            TwoComputers::SESSION,
        );
        (world, wanted)
    }

    fn accepted_sessions(link: &ProjectStorageLink) -> usize {
        link.conversation_checkpoints
            .values()
            .map(BTreeMap::len)
            .sum()
    }

    /// One session of two is pulled alone, into a project that never
    /// reviewed the storage. Its review holds its row and no file action,
    /// Apply refuses whatever is outside it, and nothing of the link moves
    /// but the session's own acceptance: the reviewed base, the cloud pin
    /// and the saved recipe stay, the other session stays only in storage,
    /// and a Push is refused until the project's Pull as it was before.
    fn a_session_pull_brings_one_session_and_moves_nothing_else(provider: &'static str) {
        let (world, wanted) = two_stored_sessions(provider);
        assert!(push_requires_pull(&world, &world.second), "{provider}");

        let service = ProjectSyncService::new(&world.repo);
        let bindings = world.repo.load_bindings().unwrap();
        let project_plan = service
            .plan_bundle_restore(
                &world.storage,
                &world.bundle_id,
                bindings.active_for(&world.second.local_project_id).unwrap(),
            )
            .unwrap();
        assert_eq!(project_plan.scope, PullScope::Project, "{provider}");
        assert_eq!(project_plan.conversation_review.len(), 2, "{provider}");
        assert!(project_plan.project_content_eligibility.is_some());

        let plan = session_pull_plan(&world.repo, &world.storage, &world.second, &wanted).unwrap();
        assert_eq!(
            plan.scope,
            PullScope::Session {
                resource_id: wanted.clone()
            }
        );
        assert!(plan.actions.is_empty(), "{provider}: {:?}", plan.actions);
        assert!(
            plan.project_content_eligibility.is_none(),
            "{provider}: a session Pull has no Files step"
        );
        assert_eq!(plan.conversation_review.len(), 1, "{provider}");
        let row = &plan.conversation_review[0];
        assert_eq!(row.action, ConversationReviewAction::CreateLocal);
        assert_eq!(row.resource_id, wanted);
        let row_id = row.action_id.clone().unwrap();

        // Whatever is outside the row is refused before anything is written.
        let before = link_of(&world.repo, &world.second);
        let other = project_plan
            .conversation_review
            .iter()
            .find(|row| row.resource_id != wanted)
            .unwrap();
        let other_id = other.action_id.clone().unwrap();
        assert!(service
            .apply_bundle_restore(&plan, vec![other_id.clone()])
            .unwrap_err()
            .contains("is not in the comparison"));
        let mut widened = plan.clone();
        widened.conversation_review.push(other.clone());
        assert!(service
            .apply_bundle_restore(&widened, vec![row_id.clone(), other_id])
            .unwrap_err()
            .contains("only its own session"));
        let mut with_files = plan.clone();
        with_files.project_content_eligibility = project_plan.project_content_eligibility.clone();
        assert!(service
            .apply_bundle_restore(&with_files, vec![row_id.clone()])
            .unwrap_err()
            .contains("no file actions"));
        assert_eq!(link_of(&world.repo, &world.second), before, "{provider}");
        assert!(world
            .repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .iter()
            .all(|row| row.local_project_id != world.second.local_project_id));
        assert_eq!(
            find_project_session_with_repository(
                &world.repo,
                &world.second.local_project_id,
                &wanted
            ),
            Ok(None),
            "{provider}: the project does not hold the session yet"
        );

        let applied = service
            .apply_bundle_restore(&plan, vec![row_id.clone()])
            .unwrap();
        assert!(applied.success, "{provider}: {applied:?}");
        assert_eq!(applied.applied_action_ids, [row_id]);
        assert_eq!(
            applied.reviewed_base_advanced, None,
            "{provider}: the rest of the storage was not reviewed"
        );

        // The link holds one acceptance more, and is otherwise as it was.
        let after = link_of(&world.repo, &world.second);
        assert_eq!(after.scope_bases, before.scope_bases, "{provider}");
        assert_eq!(after.recipe, before.recipe, "{provider}");
        assert_eq!(
            after.managed_cloud_repository_id, before.managed_cloud_repository_id,
            "{provider}"
        );
        assert_eq!(accepted_sessions(&after), accepted_sessions(&before) + 1);
        let mut but_the_acceptance = after.clone();
        but_the_acceptance.conversation_checkpoints = before.conversation_checkpoints.clone();
        assert_eq!(but_the_acceptance, before, "{provider}");
        let record = world
            .repo
            .load_materializations()
            .unwrap()
            .records
            .into_iter()
            .find(|record| Some(&record.plan_id) == applied.plan_id.as_ref())
            .expect("the session Pull has its receipts");
        assert_eq!(record.status, MaterializationStatus::Partial, "{provider}");

        // The session is here and synced; the other is still only in storage,
        // and the project's own review still offers it.
        let entries = entries_of_project(&world, &world.second);
        // Its page is opened by the id Activity lists it under.
        assert_eq!(
            find_project_session_with_repository(
                &world.repo,
                &world.second.local_project_id,
                &wanted
            )
            .unwrap()
            .as_ref(),
            entries
                .iter()
                .find(|entry| entry.local_present)
                .map(|entry| &entry.conversation_resource_id),
            "{provider}"
        );
        // The computer that made it holds it under its own id.
        assert!(find_project_session_with_repository(
            &world.repo,
            &world.first.local_project_id,
            &wanted
        )
        .unwrap()
        .is_some_and(|page| Some(&page)
            != entries
                .iter()
                .find(|entry| entry.local_present)
                .map(|entry| &entry.conversation_resource_id)));
        let mut states = entries
            .iter()
            .map(|entry| {
                (
                    entry.resource_id == wanted || entry.local_present,
                    entry.state,
                )
            })
            .collect::<Vec<_>>();
        states.sort_by_key(|(here, _)| !here);
        assert_eq!(
            states,
            [
                (true, ThreadSyncState::Synced),
                (false, ThreadSyncState::StorageOnly)
            ],
            "{provider}: {entries:?}"
        );
        let again = session_pull_plan(&world.repo, &world.storage, &world.second, &wanted).unwrap();
        assert!(
            again.conversation_review.is_empty(),
            "{provider}: already here and up to date: {:?}",
            again.conversation_review
        );
        let bindings = world.repo.load_bindings().unwrap();
        let remaining = service
            .plan_bundle_restore(
                &world.storage,
                &world.bundle_id,
                bindings.active_for(&world.second.local_project_id).unwrap(),
            )
            .unwrap();
        assert_eq!(
            remaining
                .conversation_review
                .iter()
                .map(|row| row.action)
                .collect::<Vec<_>>(),
            [ConversationReviewAction::CreateLocal],
            "{provider}"
        );

        // The rest of the storage is still to review: Push stays refused.
        assert!(push_requires_pull(&world, &world.second), "{provider}");
    }

    /// The project reviewed the storage's version and took no session. A
    /// session pulled alone afterwards leaves the base exactly where the
    /// review put it, and, continued here, is pushed from its page.
    fn a_session_pulled_into_a_reviewed_project_is_pushed_from_its_page(provider: &'static str) {
        let (world, wanted) = two_stored_sessions(provider);
        let service = ProjectSyncService::new(&world.repo);
        let bindings = world.repo.load_bindings().unwrap();
        let project_plan = service
            .plan_bundle_restore(
                &world.storage,
                &world.bundle_id,
                bindings.active_for(&world.second.local_project_id).unwrap(),
            )
            .unwrap();
        let reviewed = service
            .apply_bundle_restore(&project_plan, Vec::new())
            .unwrap();
        assert_eq!(reviewed.reviewed_base_advanced, Some(true), "{provider}");
        assert!(!push_requires_pull(&world, &world.second), "{provider}");
        let before = link_of(&world.repo, &world.second);
        assert!(!before.scope_bases.is_empty(), "{provider}");

        let plan = session_pull_plan(&world.repo, &world.storage, &world.second, &wanted).unwrap();
        assert_eq!(plan.conversation_review.len(), 1, "{provider}");
        let applied = service
            .apply_bundle_restore(
                &plan,
                plan.conversation_review
                    .iter()
                    .filter_map(|row| row.action_id.clone())
                    .collect(),
            )
            .unwrap();
        assert!(applied.success, "{provider}: {applied:?}");
        let mut after = link_of(&world.repo, &world.second);
        assert_eq!(accepted_sessions(&after), accepted_sessions(&before) + 1);
        after.conversation_checkpoints = before.conversation_checkpoints.clone();
        assert_eq!(
            after, before,
            "{provider}: the base is where the review put it"
        );
        assert!(!push_requires_pull(&world, &world.second), "{provider}");

        // Continued here, it is pushed from its page.
        let session = world.binding(&world.second).provider_thread_id.unwrap();
        let main = TwoComputers::file_of(&world.second_home, &session);
        if provider == "codex" {
            append_owned_message(&main, "continued-after-the-session-pull");
        } else {
            ask_once_more(
                &main,
                &session,
                &world.second_root,
                "continued-after-the-session-pull",
            );
        }
        let entries = entries_of_project(&world, &world.second);
        let local = entries.iter().find(|entry| entry.local_present).unwrap();
        assert_eq!(local.state, ThreadSyncState::LocalAhead, "{provider}");
        let review = service
            .prepare_push_review_with_phases(
                &world.second.local_project_id,
                &world.storage,
                PushScope::Session {
                    resource_id: local.resource_id.clone(),
                },
                &mut |_, _| {},
            )
            .unwrap();
        let pushed = service
            .push_reviewed(
                &world.second.local_project_id,
                &world.storage,
                PushApproval::recommended(&review, &world.storage),
            )
            .unwrap();
        assert!(pushed.success, "{provider}: {pushed:?}");
        let mut states = entries_of_project(&world, &world.second)
            .iter()
            .map(|entry| (entry.local_present, entry.state))
            .collect::<Vec<_>>();
        states.sort_by_key(|(here, _)| !here);
        assert_eq!(
            states,
            [
                (true, ThreadSyncState::Synced),
                (false, ThreadSyncState::StorageOnly)
            ],
            "{provider}"
        );
    }

    /// A session the storage's current version does not hold is refused
    /// when the review is asked for: nothing is compared and nothing opens.
    fn a_session_pull_of_a_session_that_is_not_stored_is_refused(provider: &'static str) {
        let (world, stored) = two_stored_sessions(provider);
        let (_, cloud_thread_id) =
            mallard_domain::parse_cloud_conversation_resource_id(stored.as_str()).unwrap();
        let other_provider = if provider == "codex" {
            "claude"
        } else {
            "codex"
        };
        for absent in [
            format!("{provider}:conversation:cloudthread-0000"),
            // The cloud thread of another provider's scope is not this one.
            format!("{other_provider}:conversation:{cloud_thread_id}"),
        ] {
            let error = session_pull_plan(
                &world.repo,
                &world.storage,
                &world.second,
                &ResourceId::parse(absent.clone()).unwrap(),
            )
            .unwrap_err();
            assert!(
                error.starts_with("That session isn't in ") && error.ends_with(" any more."),
                "{provider}: {absent}: {error}"
            );
        }
        // A session's scope is a stored row's id, never a local session's.
        for malformed in [
            format!("{provider}:session:{}", TwoComputers::SESSION),
            format!("{provider}:conversation:thread-0000"),
        ] {
            assert!(
                session_pull_plan(
                    &world.repo,
                    &world.storage,
                    &world.second,
                    &ResourceId::parse(malformed.clone()).unwrap(),
                )
                .is_err(),
                "{provider}: {malformed}"
            );
        }
        assert!(world
            .repo
            .load_materializations()
            .unwrap()
            .records
            .iter()
            .all(|record| record.local_project_id != world.second.local_project_id));
    }

    /// Changed here and in storage: the one row is Keep both, and it
    /// preserves as the project's Pull does, without moving the base.
    fn a_session_pull_of_a_session_changed_on_both_sides_keeps_both(provider: &'static str) {
        let fixture = Fixture::long(provider);
        fixture.publish_and_warm();
        let tracked = fixture.binding().cloud_thread_id.unwrap();
        fixture.edit_with_growth_is_refused_once("hello", "world", 3);
        let wanted =
            cloud_conversation_resource_id(&ProviderKey::parse(provider).unwrap(), &tracked)
                .unwrap();
        let plan =
            session_pull_plan(&fixture.repo, &fixture.storage, &fixture.project, &wanted).unwrap();
        assert_eq!(
            plan.conversation_review
                .iter()
                .map(|row| row.action)
                .collect::<Vec<_>>(),
            [ConversationReviewAction::KeepBoth],
            "{provider}"
        );
        assert!(plan.actions.is_empty(), "{provider}");
        let before = link_of(&fixture.repo, &fixture.project);
        let applied = ProjectSyncService::new(&fixture.repo)
            .apply_bundle_restore(
                &plan,
                plan.conversation_review
                    .iter()
                    .filter_map(|row| row.action_id.clone())
                    .collect(),
            )
            .unwrap();
        assert!(applied.success, "{provider}: {applied:?}");
        assert_eq!(applied.preserved_threads.len(), 1, "{provider}");
        assert_eq!(applied.reviewed_base_advanced, None, "{provider}");
        let copy = fixture.binding();
        assert!(
            copy.preserved.is_some() && copy.cloud_thread_id.is_none(),
            "{provider}: {copy:?}"
        );
        let after = link_of(&fixture.repo, &fixture.project);
        assert_eq!(after.scope_bases, before.scope_bases, "{provider}");
        assert_eq!(after.recipe, before.recipe, "{provider}");
        assert_eq!(
            Fixture::sorted_states(&fixture.refresh()),
            ["LocalOnly", "Synced", "Synced"],
            "{provider}"
        );
    }

    /// A folder that is not a Git repository: the session comes alone. No
    /// project file is written, and the project's own review still lists the
    /// stored files afterwards.
    #[test]
    fn a_session_pull_into_a_folder_writes_no_project_file() {
        // `TwoComputers::of`, with projects that name no Git repository.
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage = StorageId::parse("session-pull-folder").unwrap();
        add_local_storage(&repo, &storage, &temp.path().join("store"));
        let bundle_id = BundleId::parse("df29babc833808e68ad0efa4d01d7d6e").unwrap();
        let register = |name: &str| {
            register_local_project_with_repository(
                &repo,
                RegisterLocalProjectRequest {
                    display_name: name.to_string(),
                    repository_fingerprint: None,
                    bundle_id: Some(bundle_id.clone()),
                },
            )
            .unwrap()
        };
        let first = register("first");
        let (first_root, first_home) = (temp.path().join("proj-a"), temp.path().join("home-a"));
        bind_and_link_claude_replica(&repo, &first, &storage, &first_root, &first_home);
        let second = register("second");
        let (second_root, second_home) = (temp.path().join("proj-b"), temp.path().join("home-b"));
        bind_and_link_claude_replica(&repo, &second, &storage, &second_root, &second_home);
        let world = TwoComputers {
            temp,
            repo,
            storage,
            bundle_id,
            provider: "claude",
            first,
            first_root,
            first_home,
            second,
            second_root,
            second_home,
        };
        child_settings_fixture(
            &world.first_home,
            &world.first_root,
            "claude",
            TwoComputers::SESSION,
            0,
        );
        std::fs::write(world.first_root.join("notes.md"), "Stored project file\n").unwrap();
        let service = ProjectSyncService::new(&world.repo);
        let review = prepare_push_review_with_repository(
            &world.repo,
            &world.first.local_project_id,
            &world.storage,
        )
        .unwrap();
        let files = inspect_project_files_with_repository(
            &world.repo,
            &world.first.local_project_id,
            &world.storage,
        )
        .unwrap();
        let mut approval = PushApproval::recommended(&review, &world.storage);
        approval
            .recipe
            .entries
            .extend(reviewed_project_content_recipe(&files).entries);
        approval.project_content_review_token = files.review_token;
        let pushed = service
            .push_reviewed(&world.first.local_project_id, &world.storage, approval)
            .unwrap();
        assert!(pushed.success, "{pushed:?}");
        let (_, engine) = storage_engine(&world.repo, &world.storage).unwrap();
        assert!(
            engine
                .inspect(&world.bundle_id)
                .unwrap()
                .manifest
                .scope(&SyncScopeKey::Project)
                .is_some_and(|scope| !scope.files.is_empty()),
            "the fixture stores a project file"
        );

        let wanted = stored_session_id(
            &world.repo,
            &world.storage,
            &world.bundle_id,
            TwoComputers::SESSION,
        );
        let plan = session_pull_plan(&world.repo, &world.storage, &world.second, &wanted).unwrap();
        assert!(plan.actions.is_empty(), "{:?}", plan.actions);
        assert!(plan.project_content_eligibility.is_none());
        let applied = service
            .apply_bundle_restore(
                &plan,
                plan.conversation_review
                    .iter()
                    .filter_map(|row| row.action_id.clone())
                    .collect(),
            )
            .unwrap();
        assert!(applied.success, "{applied:?}");
        assert_eq!(applied.applied_action_ids.len(), 1);
        assert!(world.binding(&world.second).provider_thread_id.is_some());
        assert!(
            !world.second_root.join("notes.md").exists(),
            "a session Pull writes no project file"
        );

        let bindings = world.repo.load_bindings().unwrap();
        let project_plan = service
            .plan_bundle_restore(
                &world.storage,
                &world.bundle_id,
                bindings.active_for(&world.second.local_project_id).unwrap(),
            )
            .unwrap();
        assert!(
            project_plan
                .actions
                .iter()
                .any(|action| matches!(&action.kind, RestoreActionKind::WriteProjectFile { .. })),
            "the project's Files step still lists the stored file: {:?}",
            project_plan.actions
        );
        assert!(project_plan.conversation_review.is_empty());
    }

    mod session_pull_codex {
        provider_cases!("codex";
            a_session_pull_brings_one_session_and_moves_nothing_else,
            a_session_pulled_into_a_reviewed_project_is_pushed_from_its_page,
            a_session_pull_of_a_session_that_is_not_stored_is_refused,
            a_session_pull_of_a_session_changed_on_both_sides_keeps_both,
        );
    }

    mod session_pull_claude {
        provider_cases!("claude";
            a_session_pull_brings_one_session_and_moves_nothing_else,
            a_session_pulled_into_a_reviewed_project_is_pushed_from_its_page,
            a_session_pull_of_a_session_that_is_not_stored_is_refused,
            a_session_pull_of_a_session_changed_on_both_sides_keeps_both,
        );
    }
