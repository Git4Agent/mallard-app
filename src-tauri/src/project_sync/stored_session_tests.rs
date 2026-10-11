    // A stored session's page without a Pull (docs/PLAN_STORED_SESSION_DETAIL.md).
    // Synthetic provider homes only. Included in session_refresh_regressions.

    fn whole_stored_timeline(snapshot_id: &str) -> ProjectSessionTimelineRequest {
        ProjectSessionTimelineRequest {
            snapshot_id: snapshot_id.to_string(),
            cursor: None,
            limit: Some(50),
            included_event_kinds: [
                SessionTimelineEventKind::Prompt,
                SessionTimelineEventKind::Response,
                SessionTimelineEventKind::IntermediateStep,
                SessionTimelineEventKind::Checkpoint,
                SessionTimelineEventKind::ToolCall,
                SessionTimelineEventKind::AgentCall,
            ]
            .into_iter()
            .collect(),
            included_tool_categories: [
                SessionToolCategory::FileEdit,
                SessionToolCategory::Shell,
                SessionToolCategory::Read,
                SessionToolCategory::Other,
            ]
            .into_iter()
            .collect(),
            turn_id: None,
        }
    }

    /// The one file under `root` named `name`.
    fn file_named(root: &Path, name: &str) -> Option<PathBuf> {
        for entry in std::fs::read_dir(root).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = file_named(&path, name) {
                    return Some(found);
                }
            } else if path.file_name().is_some_and(|file| file == name) {
                return Some(path);
            }
        }
        None
    }

    /// The transcript object the manifest names for the stored row `wanted`.
    fn stored_transcript_object(world: &TwoComputers, wanted: &ResourceId) -> PathBuf {
        let (_, engine) = storage_engine(&world.repo, &world.storage).unwrap();
        let snapshot = engine.inspect(&world.bundle_id).unwrap();
        let thread = snapshot
            .manifest
            .all_conversations()
            .find(|(_, _, thread)| &thread.resource_id == wanted)
            .map(|(_, _, thread)| thread)
            .expect("the session is stored");
        let transcript = thread
            .files
            .iter()
            .find(|file| file.role == ConversationFileRole::Transcript)
            .expect("a transcript file");
        let (_, entry) = snapshot
            .manifest
            .find_file(&transcript.logical_path)
            .expect("the manifest lists it");
        file_named(&world.store(), &entry.sha256).expect("the object is on disk")
    }

    /// `two_stored_sessions`, with a prompt the index can count: Codex counts
    /// a prompt from its display record, which the shared fixture leaves
    /// out, so the first computer's rollout gets one before the Push.
    fn two_stored_sessions_with_a_prompt(provider: &'static str) -> (TwoComputers, ResourceId) {
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
        if provider == "codex" {
            let main = TwoComputers::file_of(&world.first_home, TwoComputers::SESSION);
            let mut file = std::fs::OpenOptions::new().append(true).open(&main).unwrap();
            writeln!(
                file,
                "{}",
                serde_json::json!({
                    "ordinal": 2,
                    "timestamp": "2026-07-22T00:00:02Z",
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": "test owned child"},
                })
            )
            .unwrap();
        }
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

    /// The first computer's own row of `TwoComputers::SESSION`: the local
    /// page the stored page is held against.
    fn local_session_id(world: &TwoComputers) -> ResourceId {
        entries_of_project(world, &world.first)
            .into_iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some(TwoComputers::SESSION))
            .map(|entry| ResourceId::parse(entry.conversation_resource_id).unwrap())
            .expect("the first computer holds the session")
    }

    fn event_kinds(
        events: &[mallard_infrastructure::providers::SessionTimelineEvent],
    ) -> Vec<SessionTimelineEventKind> {
        events.iter().map(|event| event.kind).collect()
    }

    fn sorted_jsonl_files(home: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        jsonl_files_under(home, &mut files);
        files.sort();
        files
    }

    /// The second computer never pulled, yet the stored session's page opens
    /// and reads as the first computer's local page of the same session:
    /// detail, pages and event bodies read the storage's copy, each request
    /// afresh and all to one snapshot. Nothing lands in the second computer's
    /// provider home and nothing of its link moves. Only a stored id opens
    /// this way, and only with its storage.
    fn a_stored_session_opens_without_a_pull(provider: &'static str) {
        let (world, wanted) = two_stored_sessions_with_a_prompt(provider);
        let service = ProjectSyncService::new(&world.repo);
        let files_before = sorted_jsonl_files(&world.second_home);
        let accepted_before = accepted_sessions(&link_of(&world.repo, &world.second));
        let local_id = local_session_id(&world);
        let local = service
            .get_project_session_detail(&world.first.local_project_id, &local_id)
            .unwrap_or_else(|error| panic!("{provider}: {error:?}"));

        let detail = service
            .get_stored_session_detail(&world.second.local_project_id, &world.storage, &wanted)
            .unwrap_or_else(|error| panic!("{provider}: {error:?}"));
        assert_eq!(detail.resource_id, wanted.as_str(), "{provider}");
        assert_eq!(detail.provider.as_str(), provider);
        assert_eq!(detail.provider_session_id, TwoComputers::SESSION, "{provider}");
        assert!(
            detail.actions.is_empty(),
            "{provider}: a stored session offers no launch verbs"
        );
        // The same session, read from the storage, counts as it does at home.
        assert!(
            detail
                .event_counts
                .prompts
                .value
                .is_some_and(|prompts| prompts >= 1),
            "{provider}: {:?}",
            detail.event_counts
        );
        for (stored, home) in [
            (&detail.event_counts.prompts, &local.event_counts.prompts),
            (&detail.event_counts.responses, &local.event_counts.responses),
            (&detail.event_counts.tool_calls, &local.event_counts.tool_calls),
            (&detail.event_counts.agent_calls, &local.event_counts.agent_calls),
            (&detail.tokens, &local.tokens),
        ] {
            assert_eq!(stored, home, "{provider}");
        }

        let page = service
            .get_stored_session_timeline_page(
                &world.second.local_project_id,
                &world.storage,
                &wanted,
                whole_stored_timeline(&detail.snapshot_id),
            )
            .unwrap_or_else(|error| panic!("{provider}: {error:?}"));
        assert_eq!(page.snapshot_id, detail.snapshot_id, "{provider}");
        assert!(
            page.events
                .iter()
                .any(|event| event.kind == SessionTimelineEventKind::Prompt),
            "{provider}: {:?}",
            page.events
        );
        let local_page = service
            .get_project_session_timeline_page(
                &world.first.local_project_id,
                &local_id,
                whole_stored_timeline(&local.snapshot_id),
            )
            .unwrap();
        assert_eq!(
            event_kinds(&page.events),
            event_kinds(&local_page.events),
            "{provider}"
        );
        // Each request prepares the transcript afresh and still names the
        // same snapshot: the stored object's digest, not a file's clock.
        let again = service
            .get_stored_session_detail(&world.second.local_project_id, &world.storage, &wanted)
            .unwrap();
        assert_eq!(again.snapshot_id, detail.snapshot_id, "{provider}");
        // A page of another snapshot is refused the way a local page's is.
        let stale = service
            .get_stored_session_timeline_page(
                &world.second.local_project_id,
                &world.storage,
                &wanted,
                whole_stored_timeline(&"0".repeat(64)),
            )
            .unwrap_err();
        assert_eq!(
            stale.code,
            SessionDetailErrorCode::SnapshotChanged,
            "{provider}: {stale:?}"
        );
        // An event the index does not hold is refused after the same checks.
        let missing = service
            .get_stored_session_event_detail(
                &world.second.local_project_id,
                &world.storage,
                &wanted,
                &detail.snapshot_id,
                "no-such-event",
            )
            .unwrap_err();
        assert_eq!(
            missing.code,
            SessionDetailErrorCode::InvalidRequest,
            "{provider}: {missing:?}"
        );

        assert_eq!(
            sorted_jsonl_files(&world.second_home),
            files_before,
            "{provider}: nothing was installed"
        );
        assert_eq!(
            accepted_sessions(&link_of(&world.repo, &world.second)),
            accepted_before,
            "{provider}: nothing of the link moved"
        );

        // A local row's id names no stored session.
        let mut local_entries = entries_of_project(&world, &world.first);
        let local = ResourceId::parse(local_entries.remove(0).conversation_resource_id).unwrap();
        let refused = service
            .get_stored_session_detail(&world.second.local_project_id, &world.storage, &local)
            .unwrap_err();
        assert_eq!(
            refused.code,
            SessionDetailErrorCode::InvalidRequest,
            "{provider}: {refused:?}"
        );
        // A stored id without its storage names no local session.
        let refused = service
            .get_project_session_detail(&world.second.local_project_id, &wanted)
            .unwrap_err();
        assert_eq!(
            refused.code,
            SessionDetailErrorCode::InvalidRequest,
            "{provider}: {refused:?}"
        );
    }

    #[test]
    fn a_stored_codex_session_opens_without_a_pull() {
        a_stored_session_opens_without_a_pull("codex");
    }

    #[test]
    fn a_stored_claude_session_opens_without_a_pull() {
        a_stored_session_opens_without_a_pull("claude");
    }

    #[cfg(windows)]
    #[test]
    fn stored_sessions_open_and_pull_with_a_verbatim_windows_binding() {
        for provider in ["codex", "claude"] {
            let (world, wanted) = two_stored_sessions_with_a_prompt(provider);
            world.repo
                .mutate_bindings(|_, state| {
                    let binding = state.bindings.iter_mut()
                        .find(|binding| binding.local_project_id == world.second.local_project_id)
                        .unwrap();
                    binding.project_root = std::fs::canonicalize(&binding.project_root).unwrap()
                        .to_string_lossy().into_owned();
                    assert!(binding.project_root.starts_with(r"\\?\"));
                    assert!(!binding.canonical_project_root.starts_with(r"\\?\"));
                    Ok(())
                })
                .unwrap();
            let saved = world.repo.load_bindings().unwrap();
            let service = ProjectSyncService::new(&world.repo);
            let detail = service
                .get_stored_session_detail(&world.second.local_project_id, &world.storage, &wanted)
                .unwrap_or_else(|error| panic!("{provider}: {error:?}"));
            let page = service
                .get_stored_session_timeline_page(
                    &world.second.local_project_id,
                    &world.storage,
                    &wanted,
                    whole_stored_timeline(&detail.snapshot_id),
                )
                .unwrap_or_else(|error| panic!("{provider}: {error:?}"));
            assert!(!page.events.is_empty(), "{provider}");
            assert_eq!(world.repo.load_bindings().unwrap(), saved);
            let pulled = world.pull(&world.second);
            assert!(pulled.success, "{provider}: {pulled:?}");
        }
    }

    /// A stored object that no longer matches its manifest entry is refused
    /// before anything of it is shown: the page shows verified bytes only.
    fn a_changed_stored_object_is_refused(provider: &'static str) {
        let (world, wanted) = two_stored_sessions(provider);
        let service = ProjectSyncService::new(&world.repo);
        service
            .get_stored_session_detail(&world.second.local_project_id, &world.storage, &wanted)
            .unwrap_or_else(|error| panic!("{provider}: {error:?}"));
        let object = stored_transcript_object(&world, &wanted);
        let mut bytes = std::fs::read(&object).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x20;
        std::fs::write(&object, bytes).unwrap();
        ObjectCache::global().clear();
        let refused = service
            .get_stored_session_detail(&world.second.local_project_id, &world.storage, &wanted)
            .unwrap_err();
        assert_eq!(
            refused.code,
            SessionDetailErrorCode::ProviderError,
            "{provider}: {refused:?}"
        );
    }

    #[test]
    fn a_changed_stored_codex_object_is_refused() {
        a_changed_stored_object_is_refused("codex");
    }

    #[test]
    fn a_changed_stored_claude_object_is_refused() {
        a_changed_stored_object_is_refused("claude");
    }
