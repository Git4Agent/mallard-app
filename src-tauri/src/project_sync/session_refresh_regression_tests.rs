// These tests call Refresh directly. Session discovery and observation reuse
// have no watcher registration, queue, or event input. No sleep, callback, or
// writer close is required to make a mutation visible.
mod session_refresh_regressions {
    use super::*;
    use std::io::Write;

    #[test]
    fn metadata_only_codex_session_is_absent_from_activity_and_push_until_started() {
        let fixture = Fixture::new("codex");
        let empty_id = "019fb019-0000-7000-8000-000000000003";
        let path = fixture.path_for(empty_id);
        let empty = command_test_transcript(empty_id, &fixture.root, &[]);
        std::fs::write(&path, &empty).unwrap();
        let activity = || {
            get_project_activity_with_repository(
                &fixture.repo,
                &fixture.project.local_project_id,
                Some(&fixture.storage),
                None,
                None,
                None,
                true,
            )
            .unwrap()
        };
        assert!(activity()
            .sessions
            .iter()
            .all(|session| session.provider_session_id.as_deref() != Some(empty_id)));
        assert!(fixture
            .refresh()
            .entries
            .iter()
            .all(|entry| { entry.provider_thread_id.as_deref() != Some(empty_id) }));
        let review = prepare_push_review_with_repository(
            &fixture.repo,
            &fixture.project.local_project_id,
            &fixture.storage,
        )
        .unwrap();
        assert!(review.inventory.resources.iter().all(|resource| {
            resource.descriptor.resource_id.as_str() != format!("codex:session:{empty_id}")
        }));
        assert!(fixture.push().success);
        let (_, engine) = storage_engine(&fixture.repo, &fixture.storage).unwrap();
        let snapshot = engine.inspect(&fixture.project.bundle_id).unwrap();
        assert!(snapshot
            .manifest
            .all_conversations()
            .all(|(_, _, thread)| { thread.source_thread_id.as_deref() != Some(empty_id) }));
        assert_eq!(std::fs::read(&path).unwrap(), empty);

        let started = command_test_transcript(
            empty_id,
            &fixture.root,
            &[("first-message", "user", "Hello")],
        );
        assert!(started.starts_with(&empty));
        let mut writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writer.write_all(&started[empty.len()..]).unwrap();
        writer.flush().unwrap();
        assert!(activity()
            .sessions
            .iter()
            .any(|session| session.provider_session_id.as_deref() == Some(empty_id)));
        let refreshed = fixture.refresh();
        let entry = fixture.entry(&refreshed, empty_id);
        assert_eq!(entry.state, ThreadSyncState::LocalOnly);
        assert!(entry.push_decision.selectable);
        drop(writer);
    }

    struct Fixture {
        temp: tempfile::TempDir,
        repo: LocalStateRepository,
        storage: StorageId,
        project: LocalProjectRegistration,
        root: PathBuf,
        home: PathBuf,
        provider: &'static str,
        active: &'static str,
        quiet: &'static str,
        path: PathBuf,
        /// Repetitions of a filler phrase in every message text.
        padding: usize,
    }

    impl Fixture {
        fn new(provider: &'static str) -> Self {
            Self::with_padding(provider, 0)
        }

        /// Transcripts several times longer than a cursor's probe window, so
        /// their first message lies outside it.
        fn long(provider: &'static str) -> Self {
            Self::with_padding(provider, 1_000)
        }

        fn with_padding(provider: &'static str, padding: usize) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let repo = repository(&temp);
            let storage = StorageId::parse("refresh-regression-store").unwrap();
            add_local_storage(&repo, &storage, &temp.path().join("store"));
            let project = register(&repo);
            let root = temp.path().join("project");
            let home = temp.path().join("provider-home");
            if provider == "codex" {
                bind_and_link_test_replica(&repo, &project, &storage, &root, &home);
            } else {
                bind_and_link_claude_replica(&repo, &project, &storage, &root, &home);
            }
            let mut fixture = Self {
                temp,
                repo,
                storage,
                project,
                root,
                home,
                provider,
                active: "019fb019-0000-7000-8000-000000000001",
                quiet: "019fb019-0000-7000-8000-000000000002",
                path: PathBuf::new(),
                padding,
            };
            fixture.path = fixture.path_for(fixture.active);
            for id in [fixture.active, fixture.quiet] {
                let path = fixture.path_for(id);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, fixture.transcript(id, 2)).unwrap();
            }
            fixture
        }

        fn path_for(&self, id: &str) -> PathBuf {
            if self.provider == "codex" {
                self.home
                    .join("sessions/2026/09/28")
                    .join(format!("rollout-2026-09-28T00-00-00-{id}.jsonl"))
            } else {
                self.home
                    .join("projects/source")
                    .join(format!("{id}.jsonl"))
            }
        }

        fn transcript(&self, id: &str, turns: usize) -> Vec<u8> {
            let [hello, answer, more, again, then] = ["hello", "answer", "more", "again", "then"]
                .map(|word| format!("{word}{}", " filler".repeat(self.padding)));
            if self.provider == "codex" {
                command_test_transcript(
                    id,
                    &self.root,
                    &[
                        ("m1", "user", hello.as_str()),
                        ("m2", "assistant", answer.as_str()),
                        ("m3", "user", more.as_str()),
                        ("m4", "assistant", again.as_str()),
                        ("m5", "user", then.as_str()),
                    ][..turns],
                )
            } else {
                claude_test_transcript(
                    id,
                    &self.root,
                    &[
                        ("m1", None, "user", hello.as_str()),
                        ("m2", Some("m1"), "assistant", answer.as_str()),
                        ("m3", Some("m2"), "user", more.as_str()),
                        ("m4", Some("m3"), "assistant", again.as_str()),
                        ("m5", Some("m4"), "user", then.as_str()),
                    ][..turns],
                )
            }
        }

        /// The same bytes with one word changed, far enough from the end
        /// that the probe window of a cursor at the end cannot see it.
        fn edited_before_the_probe_window(path: &Path, word: &str, other: &str) -> Vec<u8> {
            let before = String::from_utf8(std::fs::read(path).unwrap()).unwrap();
            let edit = before.find(word).expect("the fixture carries the word");
            assert!(
                (edit + word.len()) as u64 + jsonl_tail_io::PROBE_WINDOW_BYTES
                    < before.len() as u64,
                "the edit must lie before the probe window"
            );
            let changed = before.replacen(word, other, 1);
            assert_eq!(changed.len(), before.len());
            changed.into_bytes()
        }

        fn set_modified(path: &Path, modified: std::time::SystemTime) {
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(modified)
                .unwrap();
        }

        fn push(&self) -> ProjectOperationResult {
            let review = prepare_push_review_with_repository(
                &self.repo,
                &self.project.local_project_id,
                &self.storage,
            )
            .unwrap();
            ProjectSyncService::new(&self.repo)
                .push_reviewed(
                    &self.project.local_project_id,
                    &self.storage,
                    PushApproval::recommended(&review, &self.storage),
                )
                .unwrap()
        }

        /// Apply every action the Pull review offers, and return what it
        /// offered.
        fn pull(&self) -> Vec<ConversationReviewAction> {
            let service = ProjectSyncService::new(&self.repo);
            let bindings = self.repo.load_bindings().unwrap();
            let binding = bindings.active_for(&self.project.local_project_id).unwrap();
            let plan = service
                .plan_bundle_restore(&self.storage, &self.project.bundle_id, binding)
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
            assert!(applied.success, "{}: {applied:?}", self.provider);
            plan.conversation_review
                .iter()
                .map(|row| row.action)
                .collect()
        }

        fn active_checkpoint(&self, change: impl FnOnce(&mut ConversationCheckpoint)) {
            let path = self.path.canonicalize().unwrap();
            let mut change = Some(change);
            self.repo
                .transact_config(|config| {
                    for checkpoint in config
                        .links
                        .iter_mut()
                        .flat_map(|link| link.conversation_checkpoints.values_mut())
                        .flat_map(|threads| threads.values_mut())
                    {
                        let ours = matches!(
                            &checkpoint.physical,
                            PhysicalMappingState::Ready { root_mapping, .. }
                                if Path::new(&root_mapping.root_path) == path
                        );
                        if let Some(change) = change.take_if(|_| ours) {
                            change(checkpoint);
                            return Ok::<_, String>(ConfigMutation::Changed(()));
                        }
                    }
                    Ok(ConfigMutation::Unchanged(()))
                })
                .map_err(|_| "checkpoint transaction")
                .unwrap();
            assert!(change.is_none(), "{}: no checkpoint", self.provider);
        }

        fn checkpoint(&self) -> ConversationCheckpoint {
            let mut found = None;
            self.active_checkpoint(|checkpoint| found = Some(checkpoint.clone()));
            found.unwrap()
        }

        /// The binding of the session in `self.path`.
        fn binding(&self) -> LocalConversationBinding {
            self.repo
                .load_conversation_bindings()
                .unwrap()
                .bindings
                .into_iter()
                .find(|row| row.provider_thread_id.as_deref() == Some(self.active))
                .expect("the session has a binding")
        }

        /// Whether the saved origins are those of the records the file
        /// holds now, position by position. The first encoding of a Push
        /// asks the same, so a Push of such a file needs no second one.
        fn saved_origins_hold(&self) -> bool {
            self.repo
                .load_path_origins(&self.binding())
                .unwrap()
                .accepted_records_hold(
                    self.active,
                    self.active,
                    &std::fs::read(&self.path).unwrap(),
                )
        }

        fn sorted_states(comparison: &ThreadSyncComparison) -> Vec<String> {
            let mut states = comparison
                .entries
                .iter()
                .map(|entry| format!("{:?}", entry.state))
                .collect::<Vec<_>>();
            states.sort();
            states
        }

        /// Append the next turn of the fixture's transcript to the session.
        fn append_turn(&self, turns: usize) {
            let before = self.transcript(self.active, turns - 1);
            std::fs::OpenOptions::new()
                .append(true)
                .open(&self.path)
                .unwrap()
                .write_all(&self.transcript(self.active, turns)[before.len()..])
                .unwrap();
        }

        /// Change a word of the first message and add the next turn, then
        /// push: the session is refused once as edited, and is diverged
        /// from then on.
        fn edit_with_growth_is_refused_once(&self, word: &str, other: &str, turns: usize) {
            let changed = Self::edited_before_the_probe_window(&self.path, word, other);
            std::fs::write(&self.path, changed).unwrap();
            self.append_turn(turns);
            assert_eq!(
                self.entry(&self.refresh(), self.active).state,
                ThreadSyncState::LocalAhead,
                "{}",
                self.provider
            );
            let refused = self.push();
            assert_eq!(
                failed_messages(&refused),
                [owned_subagents::PUSH_EDITED],
                "{}: {refused:?}",
                self.provider
            );
            let current = self.refresh();
            assert_eq!(
                self.entry(&current, self.active).state,
                ThreadSyncState::Diverged,
                "{}",
                self.provider
            );
            self.assert_matches_fresh_observation(&current);
        }

        /// Refresh, and the file work it did under `path`.
        fn refresh_counting(
            &self,
            path: &Path,
        ) -> (
            ThreadSyncComparison,
            mallard_infrastructure::sync_metrics::MetricsSnapshot,
        ) {
            let metrics = mallard_infrastructure::sync_metrics::register_root(
                &std::fs::canonicalize(path).unwrap(),
            );
            let before = metrics.snapshot();
            let comparison = self.refresh();
            (comparison, metrics.snapshot().delta_since(&before))
        }

        fn publish_and_warm(&self) {
            let review = prepare_push_review_with_repository(
                &self.repo,
                &self.project.local_project_id,
                &self.storage,
            )
            .unwrap();
            let result = ProjectSyncService::new(&self.repo)
                .push_reviewed(
                    &self.project.local_project_id,
                    &self.storage,
                    PushApproval::recommended(&review, &self.storage),
                )
                .unwrap();
            assert!(result.success, "{}: {result:?}", self.provider);
            for _ in 0..2 {
                let comparison = self.refresh();
                assert_eq!(
                    self.entry(&comparison, self.active).state,
                    ThreadSyncState::Synced
                );
                assert_eq!(
                    self.entry(&comparison, self.quiet).state,
                    ThreadSyncState::Synced
                );
            }
        }

        fn refresh(&self) -> ThreadSyncComparison {
            let mut comparison = None;
            refresh_project_activity_with_progress(
                &self.repo,
                &self.project.local_project_id,
                &self.storage,
                None,
                None,
                true,
                &mut |event| {
                    match event {
                        ProjectActivityRefreshEvent::Comparison { comparison: result } => {
                            comparison = Some(result);
                        }
                        ProjectActivityRefreshEvent::ComparisonError { message, .. }
                        | ProjectActivityRefreshEvent::ActivityError { message, .. } => {
                            panic!("{}: {message}", self.provider);
                        }
                        _ => {}
                    }
                    Ok(())
                },
                &mut |_, _| {},
            )
            .unwrap();
            comparison.expect("Refresh must emit its comparison")
        }

        fn entry<'a>(&self, comparison: &'a ThreadSyncComparison, id: &str) -> &'a ThreadSyncEntry {
            comparison
                .entries
                .iter()
                .find(|entry| entry.provider_thread_id.as_deref() == Some(id))
                .unwrap_or_else(|| panic!("missing {id}: {:?}", comparison.entries))
        }

        fn assert_matches_fresh_observation(&self, warm: &ThreadSyncComparison) {
            super::super::super::observation_cache::observation_cache()
                .reset_and_purge_disk(self.repo.root())
                .unwrap();
            let fresh = self.refresh();
            assert_eq!(
                warm.entries, fresh.entries,
                "{}: warm and fresh observations",
                self.provider
            );
            assert_eq!(warm.counts, fresh.counts);
            assert_eq!(warm.provider_errors, fresh.provider_errors);
        }

        fn assert_changed(&self, comparison: &ThreadSyncComparison) {
            self.assert_matches_fresh_observation(comparison);
            assert_ne!(
                self.entry(comparison, self.active).state,
                ThreadSyncState::Synced,
                "{}: the old green state must not survive",
                self.provider
            );
            assert_eq!(
                self.entry(comparison, self.quiet).state,
                ThreadSyncState::Synced
            );
        }

        fn views(&self) -> Vec<provider_capture::LocalSessionView> {
            let state = self.repo.load_project_activity_state_snapshot().unwrap();
            load_project_local_read_from_state(
                self.repo.root(),
                &state,
                &self.project.local_project_id,
            )
            .unwrap()
            .views
        }

        fn assert_incomplete(&self, comparison: &ThreadSyncComparison) {
            self.assert_matches_fresh_observation(comparison);
            let entry = self.entry(comparison, self.active);
            // Invalid tail evidence maps to Unknown; a discovery rejection
            // maps to Unavailable. Both must block publication.
            assert!(
                matches!(
                    entry.state,
                    ThreadSyncState::Unknown | ThreadSyncState::Unavailable
                ),
                "{}: incomplete input must not remain green: {entry:?}",
                self.provider
            );
            assert!(!entry.push_decision.selectable);
            assert_eq!(
                self.entry(comparison, self.quiet).state,
                ThreadSyncState::Synced
            );
        }
    }

    fn failed_messages(result: &ProjectOperationResult) -> Vec<&str> {
        result
            .results
            .iter()
            .filter(|row| row.state == "failed")
            .filter_map(|row| row.message.as_deref())
            .collect()
    }

    /// The stored objects that hold `marker`.
    fn stored_objects(store: &Path, marker: &str) -> BTreeSet<Vec<u8>> {
        fn walk(directory: &Path, marker: &[u8], found: &mut BTreeSet<Vec<u8>>) {
            for entry in std::fs::read_dir(directory).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, marker, found);
                } else if let Ok(bytes) = std::fs::read(&path) {
                    if bytes.windows(marker.len()).any(|window| window == marker) {
                        found.insert(bytes);
                    }
                }
            }
        }
        let mut found = BTreeSet::new();
        walk(store, marker.as_bytes(), &mut found);
        found
    }

    /// The rows of the one stored transcript that holds `marker`.
    fn stored_rows(store: &Path, marker: &str) -> Vec<serde_json::Value> {
        let mut found = stored_objects(store, marker);
        assert_eq!(found.len(), 1, "stored transcripts that hold {marker}");
        found
            .pop_first()
            .unwrap()
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect()
    }

    fn row_of<'a>(rows: &'a [serde_json::Value], uuid: &str) -> &'a serde_json::Value {
        rows.iter()
            .find(|row| row["uuid"] == uuid)
            .unwrap_or_else(|| panic!("no row {uuid}"))
    }

    fn refresh_sees_each_rapid_append_before_the_writer_closes(provider: &'static str) {
        let fixture = Fixture::new(provider);
        fixture.publish_and_warm();
        let mut writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.path)
            .unwrap();
        for turns in 3..=4 {
            let before = fixture.transcript(fixture.active, turns - 1);
            let after = fixture.transcript(fixture.active, turns);
            assert!(after.starts_with(&before));
            writer.write_all(&after[before.len()..]).unwrap();
            writer.sync_all().unwrap();
            let current = fixture.refresh();
            assert_eq!(
                fixture.entry(&current, fixture.active).state,
                ThreadSyncState::LocalAhead
            );
            assert_eq!(
                fixture.entry(&current, fixture.active).capture_bytes,
                Some(after.len() as u64)
            );
            fixture.assert_changed(&current);
            assert_eq!(writer.metadata().unwrap().len(), after.len() as u64);
        }
        let open = fixture.refresh();
        drop(writer);
        assert_eq!(
            open.entries,
            fixture.refresh().entries,
            "late close adds no content"
        );
    }

    fn refresh_rejects_equal_size_rewrites_truncation_and_atomic_replacement(
        provider: &'static str,
    ) {
        for mutation in ["rewrite", "truncate", "replace"] {
            let fixture = Fixture::new(provider);
            fixture.publish_and_warm();
            let before = std::fs::read(&fixture.path).unwrap();
            let changed = String::from_utf8(before.clone())
                .unwrap()
                .replace("hello", "world");
            assert_eq!(before.len(), changed.len());
            let modified = std::fs::metadata(&fixture.path)
                .unwrap()
                .modified()
                .unwrap();
            match mutation {
                "rewrite" => std::fs::write(&fixture.path, changed).unwrap(),
                "truncate" => {
                    std::fs::write(&fixture.path, fixture.transcript(fixture.active, 1)).unwrap()
                }
                _ => {
                    let replacement = fixture.temp.path().join("replacement.jsonl");
                    std::fs::write(&replacement, changed).unwrap();
                    std::fs::rename(replacement, &fixture.path).unwrap();
                }
            }
            // Preserve mtime on Unix so this also covers equal size/time
            // replacement and rewrite. SourceStamp has Unix identity/ctime.
            #[cfg(unix)]
            if mutation != "truncate" {
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&fixture.path)
                    .unwrap()
                    .set_modified(modified)
                    .unwrap();
            }
            #[cfg(not(unix))]
            let _ = modified;
            fixture.assert_changed(&fixture.refresh());
        }
    }

    // The probe window witnesses only the last bytes before the cursor. A
    // transcript that changed without growing, or is another file, is never
    // read through its cursor: it is read in full and its content decides.
    fn refresh_reads_in_full_a_long_transcript_that_changed_without_growing(
        provider: &'static str,
    ) {
        for mutation in ["rewrite", "replace", "touch"] {
            let fixture = Fixture::long(provider);
            fixture.publish_and_warm();
            let changed = Fixture::edited_before_the_probe_window(&fixture.path, "hello", "world");
            let modified = std::fs::metadata(&fixture.path)
                .unwrap()
                .modified()
                .unwrap();
            match mutation {
                "rewrite" => std::fs::write(&fixture.path, changed).unwrap(),
                "replace" => {
                    let replacement = fixture.temp.path().join("replacement.jsonl");
                    std::fs::write(&replacement, changed).unwrap();
                    std::fs::rename(replacement, &fixture.path).unwrap();
                }
                // Metadata alone: the content is what was accepted.
                _ => Fixture::set_modified(
                    &fixture.path,
                    modified + std::time::Duration::from_secs(1),
                ),
            }
            // Where a change time exists it gives the edit away although
            // size and modification time are what they were.
            #[cfg(unix)]
            if mutation != "touch" {
                Fixture::set_modified(&fixture.path, modified);
            }
            let (current, work) = fixture.refresh_counting(&fixture.path);
            assert_eq!(
                (work.full_graph_resolves, work.probe_bytes_read),
                (1, 0),
                "{provider} {mutation}: read in full, not through the cursor"
            );
            let state = &fixture.entry(&current, fixture.active).state;
            if mutation == "touch" {
                assert_eq!(*state, ThreadSyncState::Synced, "{provider}");
                assert_eq!(
                    fixture.entry(&current, fixture.quiet).state,
                    ThreadSyncState::Synced
                );
                fixture.assert_matches_fresh_observation(&current);
            } else {
                assert_eq!(*state, ThreadSyncState::Diverged, "{provider} {mutation}");
                fixture.assert_changed(&current);
            }
        }
    }

    // Refresh saves nothing, so the cursor keeps the file state it was
    // saved from. A file that has grown since is read through it again,
    // whatever happened to its metadata in between.
    fn refresh_reads_growth_through_the_cursor_after_a_full_read(provider: &'static str) {
        let fixture = Fixture::long(provider);
        fixture.publish_and_warm();
        let modified = std::fs::metadata(&fixture.path)
            .unwrap()
            .modified()
            .unwrap();
        Fixture::set_modified(&fixture.path, modified + std::time::Duration::from_secs(1));
        let (touched, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(work.full_graph_resolves, 1, "{provider}");
        assert_eq!(
            fixture.entry(&touched, fixture.active).state,
            ThreadSyncState::Synced
        );

        let before = fixture.transcript(fixture.active, 2);
        let after = fixture.transcript(fixture.active, 3);
        let mut writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.path)
            .unwrap();
        writer.write_all(&after[before.len()..]).unwrap();
        writer.sync_all().unwrap();
        let (current, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(work.full_graph_resolves, 0, "{provider}");
        assert_eq!(
            work.probe_bytes_read,
            jsonl_tail_io::PROBE_WINDOW_BYTES,
            "{provider}"
        );
        assert_eq!(
            work.append_bytes_read,
            (after.len() - before.len()) as u64,
            "{provider}"
        );
        assert_eq!(
            fixture.entry(&current, fixture.active).state,
            ThreadSyncState::LocalAhead
        );
        fixture.assert_changed(&current);
        drop(writer);
    }

    // The boundary of the cursor rule, and what closes it. An edit that
    // comes with growth passes the rule, and the probe window cannot see an
    // edit that far back: Refresh reads the growth through the cursor. Push
    // reads the whole session and finds the changed record. It refuses the
    // session once and stops its cursor from being used, so the next
    // Refresh reads in full and checks the same digests.
    fn an_edit_that_came_with_growth_is_refused_once_and_then_diverged(provider: &'static str) {
        let fixture = Fixture::long(provider);
        fixture.publish_and_warm();
        let accepted = fixture.checkpoint();
        let mut changed = Fixture::edited_before_the_probe_window(&fixture.path, "hello", "world");
        let before = fixture.transcript(fixture.active, 2);
        changed.extend_from_slice(&fixture.transcript(fixture.active, 3)[before.len()..]);
        std::fs::write(&fixture.path, changed).unwrap();

        let (unseen, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(work.full_graph_resolves, 0, "{provider}");
        assert_eq!(
            fixture.entry(&unseen, fixture.active).state,
            ThreadSyncState::LocalAhead
        );

        let refused = fixture.push();
        assert!(!refused.success, "{provider}: {refused:?}");
        let failed = refused
            .results
            .iter()
            .filter(|row| row.state == "failed")
            .collect::<Vec<_>>();
        assert_eq!(failed.len(), 1, "{provider}: {:?}", refused.results);
        assert_eq!(
            failed[0].message.as_deref(),
            Some(owned_subagents::PUSH_EDITED)
        );

        // Everything that was accepted is still saved. The cursor alone
        // lost the file state that made it usable.
        let stopped = fixture.checkpoint();
        assert!(stopped.accepted.is_some());
        assert_eq!(stopped.accepted, accepted.accepted);
        assert_eq!(stopped.accepted_local_tail, accepted.accepted_local_tail);
        assert!(matches!(
            stopped.physical,
            PhysicalMappingState::Ready {
                source_state: None,
                ..
            }
        ));

        let (current, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(
            (work.full_graph_resolves, work.probe_bytes_read),
            (1, 0),
            "{provider}: read in full"
        );
        let entry = fixture.entry(&current, fixture.active);
        assert_eq!(entry.state, ThreadSyncState::Diverged, "{provider}");
        assert_eq!(
            entry.push_decision.action,
            ConversationReviewAction::RequirePull
        );
        assert_eq!(
            entry.pull_decision.action,
            ConversationReviewAction::KeepBoth
        );
        fixture.assert_changed(&current);

        // The verdict is kept like any other result.
        let (again, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(again.entries, current.entries);
        assert_eq!(
            (work.full_graph_resolves, work.transcript_bytes_read),
            (0, 0),
            "{provider}"
        );

        // A second Push holds the session for a Pull. It refuses nothing,
        // and it saves no cursor from the changed file.
        let held = fixture.push();
        assert!(held.success, "{provider}: {held:?}");
        assert_eq!(
            held.results
                .iter()
                .filter(|row| row.state == "held")
                .count(),
            1,
            "{provider}: {:?}",
            held.results
        );
        assert_eq!(fixture.checkpoint(), stopped);
        assert_eq!(fixture.refresh().entries, current.entries);

        // Keep both restores the stored copy and keeps the edited one.
        assert_eq!(fixture.pull(), [ConversationReviewAction::KeepBoth]);
        let mut states = fixture
            .refresh()
            .entries
            .iter()
            .map(|entry| format!("{:?}", entry.state))
            .collect::<Vec<_>>();
        states.sort();
        assert_eq!(states, ["LocalOnly", "Synced", "Synced"], "{provider}");
    }

    // The digest check is of the records, not of the way they were read. A
    // session that is read in full because its cursor has no file state,
    // as a Pull leaves it, and that has only grown, is local progress.
    fn a_session_read_in_full_that_only_grew_is_local_progress(provider: &'static str) {
        let fixture = Fixture::long(provider);
        fixture.publish_and_warm();
        fixture.active_checkpoint(|checkpoint| {
            if let PhysicalMappingState::Ready { source_state, .. } = &mut checkpoint.physical {
                *source_state = None;
            }
            checkpoint.generation += 1;
        });
        let before = fixture.transcript(fixture.active, 2);
        let after = fixture.transcript(fixture.active, 3);
        let mut writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.path)
            .unwrap();
        writer.write_all(&after[before.len()..]).unwrap();
        writer.sync_all().unwrap();

        let (current, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(
            (work.full_graph_resolves, work.probe_bytes_read),
            (1, 0),
            "{provider}"
        );
        assert_eq!(
            fixture.entry(&current, fixture.active).state,
            ThreadSyncState::LocalAhead
        );
        fixture.assert_changed(&current);

        let pushed = fixture.push();
        assert!(pushed.success, "{provider}: {pushed:?}");
        assert_eq!(
            fixture.entry(&fixture.refresh(), fixture.active).state,
            ThreadSyncState::Synced
        );
        // The Push saved the file state again: growth is read through the
        // cursor from here on.
        let grown = fixture.transcript(fixture.active, 4);
        writer.write_all(&grown[after.len()..]).unwrap();
        writer.sync_all().unwrap();
        let (current, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(work.full_graph_resolves, 0, "{provider}");
        assert_eq!(
            fixture.entry(&current, fixture.active).state,
            ThreadSyncState::LocalAhead
        );
        drop(writer);
    }

    // A rewind changes which messages the session consists of and changes
    // no record. The digests hold, so it is no edit: it stays local
    // progress, as it was before the digests were checked.
    #[test]
    fn a_claude_rewind_changes_no_record_and_stays_local_progress() {
        let fixture = Fixture::long("claude");
        fixture.publish_and_warm();
        let branch = claude_test_transcript(
            fixture.active,
            &fixture.root,
            &[
                ("m3", Some("m1"), "user", "another question"),
                ("m4", Some("m3"), "assistant", "another answer"),
            ],
        );
        std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.path)
            .unwrap()
            .write_all(&branch)
            .unwrap();

        let (current, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(work.full_graph_resolves, 1, "a rewind is read in full");
        assert_eq!(
            fixture.entry(&current, fixture.active).state,
            ThreadSyncState::LocalAhead
        );
        fixture.assert_changed(&current);
        let pushed = fixture.push();
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(
            fixture.entry(&fixture.refresh(), fixture.active).state,
            ThreadSyncState::Synced
        );
    }

    // The Claude desktop app rewrites a transcript when it imports it:
    // records change, messages do not. That is no edit, for comparison and
    // for Push. A record that kept its content keeps the origin that was
    // saved for it, wherever it is now. A rewritten record is scanned as a
    // new one. Nothing is published before the accepted messages are
    // proven in both forms.
    fn a_desktop_import_is_no_edit_and_is_pushed(thinking_rows: usize) {
        let fixture = Fixture::new("claude");
        let store = fixture.temp.path().join("store");
        let row = claude_row;
        let root = fixture
            .repo
            .load_bindings()
            .unwrap()
            .active_for(&fixture.project.local_project_id)
            .unwrap()
            .project_root
            .clone();
        let mut rows = vec![row("u-1", None, "user", serde_json::json!("hello"))];
        for index in 1..=thinking_rows {
            let parent = rows.last().unwrap()["uuid"].as_str().unwrap().to_string();
            rows.push(row(
                &format!("t-{index}"),
                Some(&parent),
                "assistant",
                thinking_only_content(),
            ));
        }
        let parent = rows.last().unwrap()["uuid"].as_str().unwrap().to_string();
        rows.push(row(
            "a-1",
            Some(&parent),
            "assistant",
            serde_json::json!([
                {"type": "thinking", "thinking": "hidden", "signature": "s"},
                {"type": "text", "text": format!("see {root}/src/main.rs")}
            ]),
        ));
        rows.push(row(
            "u-2",
            Some("a-1"),
            "user",
            serde_json::json!("follow-up"),
        ));
        let base = claude_thinking_transcript(fixture.active, &fixture.root, &rows);
        std::fs::write(&fixture.path, &base).unwrap();
        fixture.publish_and_warm();
        let accepted = stored_rows(&store, r#""uuid":"u-2""#);
        assert_eq!(accepted.len(), rows.len(), "captures stay verbatim");

        let mut imported = desktop_import_bytes(&base);
        assert_ne!(imported, base);
        imported.extend_from_slice(&claude_thinking_transcript(
            fixture.active,
            &fixture.root,
            &[row(
                "u-3",
                Some("u-2"),
                "user",
                serde_json::json!("appended"),
            )],
        ));
        std::fs::write(&fixture.path, &imported).unwrap();
        assert!(!fixture.saved_origins_hold());

        let (current, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(work.full_graph_resolves, 1);
        assert_eq!(
            fixture.entry(&current, fixture.active).state,
            ThreadSyncState::LocalAhead
        );
        fixture.assert_changed(&current);

        let pushed = fixture.push();
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(failed_messages(&pushed), [""; 0], "{pushed:?}");
        let synced = fixture.refresh();
        assert_eq!(
            fixture.entry(&synced, fixture.active).state,
            ThreadSyncState::Synced
        );
        fixture.assert_matches_fresh_observation(&synced);
        assert_eq!(std::fs::read(&fixture.path).unwrap(), imported);

        // The stored transcript is the imported one. The row that lost its
        // thinking is what it was but for the thinking, its project path
        // still a reference.
        let stored = stored_rows(&store, r#""uuid":"u-3""#);
        assert_eq!(stored.len(), rows.len() - thinking_rows + 1);
        assert_eq!(
            Some(row_of(&stored, "a-1")),
            mallard_infrastructure::providers::claude::project_claude_record(
                row_of(&accepted, "a-1").clone()
            )
            .as_ref()
        );
        assert_eq!(row_of(&stored, "u-1"), row_of(&accepted, "u-1"));
        let text = row_of(&stored, "a-1")["message"]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(text.starts_with("see /__mallard_root__/"), "{text}");

        // The origins saved by that Push are those of the imported file, so
        // the next Push is an ordinary one, and growth is read through the
        // cursor again.
        assert!(fixture.saved_origins_hold());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.path)
            .unwrap()
            .write_all(&claude_thinking_transcript(
                fixture.active,
                &fixture.root,
                &[row(
                    "a-3",
                    Some("u-3"),
                    "assistant",
                    serde_json::json!([{"type": "text", "text": "done"}]),
                )],
            ))
            .unwrap();
        assert!(fixture.saved_origins_hold());
        let (grown, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(work.full_graph_resolves, 0);
        assert_eq!(
            fixture.entry(&grown, fixture.active).state,
            ThreadSyncState::LocalAhead
        );
        let pushed = fixture.push();
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(
            fixture.entry(&fixture.refresh(), fixture.active).state,
            ThreadSyncState::Synced
        );
    }

    #[test]
    fn a_desktop_import_changes_no_message_and_is_no_edit() {
        a_desktop_import_is_no_edit_and_is_pushed(1);
    }

    // The import removed more records than the session has grown by since:
    // the file holds fewer records than were saved.
    #[test]
    fn a_desktop_import_that_shortens_the_transcript_is_pushed() {
        a_desktop_import_is_no_edit_and_is_pushed(3);
    }

    // The accepted wire value is of the redaction the stored version was
    // published under. When the setting was changed since, the new wire
    // form cannot give that value, whatever happened to the records. The
    // proof is then made on one more encoding, under the redaction of the
    // stored version, and what is published follows the setting of today.
    #[test]
    fn an_imported_session_is_proven_under_the_redaction_it_was_stored_with() {
        const SECRET: &str = "sk-ant-api03-aaaabbbbccccddddeeee";
        for stored_redacted in [true, false] {
            let fixture = Fixture::new("claude");
            let store = fixture.temp.path().join("store");
            let row = claude_row;
            let base = claude_thinking_transcript(
                fixture.active,
                &fixture.root,
                &[
                    row(
                        "u-1",
                        None,
                        "user",
                        serde_json::json!(format!("my key is {SECRET}")),
                    ),
                    row("t-1", Some("u-1"), "assistant", thinking_only_content()),
                    row(
                        "a-1",
                        Some("t-1"),
                        "assistant",
                        serde_json::json!([{"type": "text", "text": "answer"}]),
                    ),
                ],
            );
            std::fs::write(&fixture.path, &base).unwrap();
            save_test_redaction(&fixture.repo, stored_redacted);
            fixture.publish_and_warm();
            let text = |rows: &[serde_json::Value]| {
                row_of(rows, "u-1")["message"]["content"]
                    .as_str()
                    .unwrap()
                    .to_string()
            };
            assert_eq!(
                text(&stored_rows(&store, r#""uuid":"a-1""#)).contains(SECRET),
                !stored_redacted
            );

            save_test_redaction(&fixture.repo, !stored_redacted);
            let mut imported = desktop_import_bytes(&base);
            imported.extend_from_slice(&claude_thinking_transcript(
                fixture.active,
                &fixture.root,
                &[row(
                    "u-2",
                    Some("a-1"),
                    "user",
                    serde_json::json!("appended"),
                )],
            ));
            std::fs::write(&fixture.path, &imported).unwrap();
            let pushed = fixture.push();
            assert!(pushed.success, "{stored_redacted}: {pushed:?}");
            assert_eq!(
                failed_messages(&pushed),
                [""; 0],
                "{stored_redacted}: {pushed:?}"
            );
            assert_eq!(
                text(&stored_rows(&store, r#""uuid":"u-2""#)).contains(SECRET),
                stored_redacted
            );
            let synced = fixture.refresh();
            assert_eq!(
                fixture.entry(&synced, fixture.active).state,
                ThreadSyncState::Synced,
                "{stored_redacted}"
            );
            fixture.assert_matches_fresh_observation(&synced);
        }
    }

    // What a Push costs when its session was imported. A release build
    // gives the figures that are worth comparing:
    //
    // cargo test --release --manifest-path src-tauri/Cargo.toml --lib -- \
    //     --ignored push_cost_sample --nocapture
    #[test]
    #[ignore = "manual cost sample of 10 MiB; debug timings are not production benchmarks"]
    fn push_cost_sample_of_an_imported_session() {
        let fixture = Fixture::new("claude");
        let row = claude_row;
        let mut rows = Vec::new();
        let mut size = 0;
        while size < 10 * 1024 * 1024 {
            let index = rows.len();
            let text = format!(
                "{} {}",
                "x".repeat(1000),
                if index % 4 == 0 {
                    format!("{}/file", fixture.root.display())
                } else {
                    "ordinary text".to_string()
                }
            );
            let thinking = serde_json::json!({
                "type": "thinking", "thinking": text, "signature": "s"
            });
            let parent = index.checked_sub(1).map(|parent| format!("r-{parent}"));
            rows.push(match index % 10 {
                3 => row(
                    &format!("r-{index}"),
                    parent.as_deref(),
                    "assistant",
                    serde_json::json!([thinking]),
                ),
                7 => row(
                    &format!("r-{index}"),
                    parent.as_deref(),
                    "assistant",
                    serde_json::json!([thinking, {"type": "text", "text": text}]),
                ),
                _ => row(
                    &format!("r-{index}"),
                    parent.as_deref(),
                    if index % 2 == 0 { "user" } else { "assistant" },
                    serde_json::json!([{"type": "text", "text": text}]),
                ),
            });
            size += rows[index].to_string().len() + 1;
        }
        let base = claude_thinking_transcript(fixture.active, &fixture.root, &rows);
        std::fs::write(&fixture.path, &base).unwrap();
        let next = std::cell::Cell::new(rows.len());
        let append = || {
            let index = next.replace(next.get() + 1);
            std::fs::OpenOptions::new()
                .append(true)
                .open(&fixture.path)
                .unwrap()
                .write_all(&claude_thinking_transcript(
                    fixture.active,
                    &fixture.root,
                    &[row(
                        &format!("r-{index}"),
                        Some(&format!("r-{}", index - 1)),
                        "user",
                        serde_json::json!("appended"),
                    )],
                ))
                .unwrap();
        };
        let push = |label: &str| {
            let started = std::time::Instant::now();
            let pushed = fixture.push();
            let elapsed = started.elapsed();
            assert!(pushed.success, "{label}: {pushed:?}");
            assert_eq!(failed_messages(&pushed), [""; 0], "{label}");
            eprintln!(
                "push cost sample: {label}: {:.0} ms, {} records, {} bytes",
                elapsed.as_secs_f64() * 1000.0,
                std::fs::read(&fixture.path)
                    .unwrap()
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count(),
                std::fs::metadata(&fixture.path).unwrap().len()
            );
        };
        push("first Push");
        for _ in 0..3 {
            append();
            push("Push of an appended session");
        }
        let imported = desktop_import_bytes(&std::fs::read(&fixture.path).unwrap());
        std::fs::write(&fixture.path, imported).unwrap();
        append();
        assert!(!fixture.saved_origins_hold());
        push("Push of an imported and appended session");
        for _ in 0..3 {
            append();
            assert!(fixture.saved_origins_hold());
            push("Push of an appended session, after the import");
        }
    }

    // The rule of the second encoding, case by case, with the encodings it
    // makes counted. No transcript is encoded again without the proof in
    // the local form, which costs no encoding. Redaction stores two keys
    // as the same text, so a message whose key was changed has the
    // accepted wire form, and the local form is what tells it apart.
    #[test]
    fn a_transcript_is_encoded_again_only_with_what_permits_it() {
        const KEYS: [&str; 2] = [
            "sk-ant-api03-aaaabbbbccccddddeeee",
            "sk-ant-api03-eeeeddddccccbbbbaaaa",
        ];
        let fixture = Fixture::new("claude");
        let row = claude_row;
        let transcript = |key: &str| {
            claude_thinking_transcript(
                fixture.active,
                &fixture.root,
                &[
                    row(
                        "u-1",
                        None,
                        "user",
                        serde_json::json!(format!("my key is {key}")),
                    ),
                    row("t-1", Some("u-1"), "assistant", thinking_only_content()),
                    row(
                        "a-1",
                        Some("t-1"),
                        "assistant",
                        serde_json::json!([{"type": "text", "text": "answer"}]),
                    ),
                ],
            )
        };
        let appended = claude_thinking_transcript(
            fixture.active,
            &fixture.root,
            &[row(
                "u-2",
                Some("a-1"),
                "user",
                serde_json::json!("appended"),
            )],
        );
        std::fs::write(&fixture.path, transcript(KEYS[0])).unwrap();
        save_test_redaction(&fixture.repo, true);
        fixture.publish_and_warm();

        let accepted = fixture.checkpoint();
        let mut unaccepted = accepted.clone();
        unaccepted.accepted = None;
        let origins = fixture.repo.load_path_origins(&fixture.binding()).unwrap();
        let bindings = fixture.repo.load_bindings().unwrap();
        let binding = bindings
            .active_for(&fixture.project.local_project_id)
            .unwrap();
        let mapper = SessionPathMapper::new(
            &binding.project_root,
            (binding.canonical_project_root != binding.project_root)
                .then_some(binding.canonical_project_root.as_str()),
            mallard_infrastructure::session_path_map::home_dir_text().as_deref(),
        )
        .unwrap();
        let redacted = || credential_redaction::shared().begin_run();
        let stored = redacted().stamp();
        let imported = [desktop_import_bytes(&transcript(KEYS[0])), appended.clone()].concat();
        let edited = [transcript(KEYS[1]), appended].concat();

        let again = |bytes: &[u8],
                     kept_copy: bool,
                     checkpoint: &ConversationCheckpoint,
                     mut redaction: credential_redaction::RedactionRun<'_>| {
            let encodings = std::cell::Cell::new(0);
            let encode = |lookup, redaction: &mut credential_redaction::RedactionRun<'_>| {
                encodings.set(encodings.get() + 1);
                mallard_infrastructure::providers::claude::publication::encode_main_transcript(
                    bytes,
                    fixture.active,
                    &mapper,
                    &origins,
                    lookup,
                    redaction,
                )
            };
            let refusal = encode(OriginLookup::ByPosition, &mut redaction)
                .err()
                .expect("the first encoding refuses a rewritten transcript");
            assert!(refusal.saved_record_changed);
            let analysis =
                mallard_infrastructure::providers::claude::analyze_claude_transcript(bytes);
            let result = encode_rewritten_claude_transcript(
                refusal.clone(),
                encode,
                &mut redaction,
                kept_copy,
                Some(checkpoint),
                analysis.lineage.chain(),
                Some(&stored),
            )
            .map(|_| ())
            .map_err(|kept| assert_eq!(kept, refusal));
            (result.is_ok(), encodings.get() - 1)
        };

        assert_eq!(again(&imported, false, &accepted, redacted()), (true, 1));
        assert_eq!(
            again(&imported, false, &accepted, redacted().with_enabled(false)),
            (true, 2),
            "the proof is made under the redaction of the stored version"
        );
        assert_eq!(again(&imported, false, &unaccepted, redacted()), (false, 0));
        assert_eq!(again(&edited, false, &accepted, redacted()), (false, 0));
        assert_eq!(again(&edited, true, &accepted, redacted()), (true, 1));
    }

    // Redaction stores two keys as the same text, so the stored form of a
    // message whose key was changed is the accepted one. The message is
    // not the accepted one, and the session is refused as edited.
    #[test]
    fn an_edit_that_redaction_hides_is_an_edit() {
        let fixture = Fixture::long("claude");
        let row = claude_row;
        let filler = " filler".repeat(fixture.padding);
        std::fs::write(
            &fixture.path,
            claude_thinking_transcript(
                fixture.active,
                &fixture.root,
                &[
                    row(
                        "u-1",
                        None,
                        "user",
                        serde_json::json!(format!(
                            "my key is sk-ant-api03-aaaabbbbccccddddeeee{filler}"
                        )),
                    ),
                    row(
                        "a-1",
                        Some("u-1"),
                        "assistant",
                        serde_json::json!([{"type": "text", "text": format!("answer{filler}")}]),
                    ),
                ],
            ),
        )
        .unwrap();
        save_test_redaction(&fixture.repo, true);
        fixture.publish_and_warm();
        let mut edited = Fixture::edited_before_the_probe_window(
            &fixture.path,
            "aaaabbbbccccddddeeee",
            "eeeeddddccccbbbbaaaa",
        );
        edited.extend_from_slice(&claude_thinking_transcript(
            fixture.active,
            &fixture.root,
            &[row(
                "u-2",
                Some("a-1"),
                "user",
                serde_json::json!("appended"),
            )],
        ));
        std::fs::write(&fixture.path, edited).unwrap();
        assert_eq!(
            fixture.entry(&fixture.refresh(), fixture.active).state,
            ThreadSyncState::LocalAhead
        );

        let refused = fixture.push();
        assert_eq!(
            failed_messages(&refused),
            [owned_subagents::PUSH_EDITED],
            "{refused:?}"
        );
        let current = fixture.refresh();
        assert_eq!(
            fixture.entry(&current, fixture.active).state,
            ThreadSyncState::Diverged
        );
        fixture.assert_matches_fresh_observation(&current);
    }

    // The first encoding of the imported session is refused, the second one
    // is published. When that publication fails, nothing of the second
    // encoding is kept: the binding names the origins it named, the
    // accepted version is the one it was, and the next Push publishes.
    #[test]
    fn a_failed_publication_of_an_imported_session_saves_nothing() {
        let fixture = Fixture::new("claude");
        let store = fixture.temp.path().join("store");
        let row = claude_row;
        let base = claude_thinking_transcript(
            fixture.active,
            &fixture.root,
            &[
                row("u-1", None, "user", serde_json::json!("hello")),
                row("t-1", Some("u-1"), "assistant", thinking_only_content()),
                row(
                    "a-1",
                    Some("t-1"),
                    "assistant",
                    serde_json::json!([{"type": "text", "text": "answer"}]),
                ),
            ],
        );
        std::fs::write(&fixture.path, &base).unwrap();
        fixture.publish_and_warm();
        let mut imported = desktop_import_bytes(&base);
        imported.extend_from_slice(&claude_thinking_transcript(
            fixture.active,
            &fixture.root,
            &[row(
                "u-2",
                Some("a-1"),
                "user",
                serde_json::json!("appended"),
            )],
        ));
        std::fs::write(&fixture.path, &imported).unwrap();
        let local_state = || {
            (
                fixture.repo.load_config().unwrap(),
                fixture.repo.load_conversation_bindings().unwrap(),
                fixture.repo.load_materializations().unwrap(),
            )
        };

        let review = prepare_push_review_with_repository(
            &fixture.repo,
            &fixture.project.local_project_id,
            &fixture.storage,
        )
        .unwrap();
        let before = local_state();
        std::thread::sleep(Duration::from_millis(5));
        mallard_infrastructure::storage_core::publication::test_support::before_next_record_request(
            {
                let (store, bundle_id) = (store.clone(), fixture.project.bundle_id.clone());
                move || {
                    let parent = record_names_at(&store, &bundle_id, 2).remove(0);
                    plant_rival_publication(
                        &store,
                        &bundle_id,
                        &parent,
                        &format!("commit-{}", "e".repeat(24)),
                        parent.created_at_ms() + 1,
                    );
                }
            },
        );
        let error = ProjectSyncService::new(&fixture.repo)
            .push_reviewed(
                &fixture.project.local_project_id,
                &fixture.storage,
                PushApproval::recommended(&review, &fixture.storage),
            )
            .unwrap_err();
        assert_eq!(
            error,
            mallard_infrastructure::storage_core::publication::REMOTE_CHANGED
        );
        // The cursor follows the file, as in every Push. What was
        // accepted, and the origins the binding names, are what they were.
        let (config, bindings, materializations) = local_state();
        assert_eq!(bindings, before.1);
        assert_eq!(materializations, before.2);
        let accepted = |config: &SyncConfig| {
            config
                .links
                .iter()
                .flat_map(|link| link.conversation_checkpoints.values())
                .flat_map(|threads| threads.iter())
                .map(|(thread, checkpoint)| {
                    (
                        thread.clone(),
                        checkpoint.accepted.clone(),
                        checkpoint.accepted_local_tail.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(accepted(&config), accepted(&before.0));
        assert!(!fixture.saved_origins_hold());

        let pushed = fixture.push();
        assert!(pushed.success, "{pushed:?}");
        assert!(fixture.saved_origins_hold());
        assert_eq!(
            fixture.entry(&fixture.refresh(), fixture.active).state,
            ThreadSyncState::Synced
        );
    }

    /// Two computers that share one storage: the first writes a session and
    /// pushes it, the second pulls it.
    struct TwoComputers {
        temp: tempfile::TempDir,
        repo: LocalStateRepository,
        storage: StorageId,
        bundle_id: BundleId,
        provider: &'static str,
        first: LocalProjectRegistration,
        first_root: PathBuf,
        first_home: PathBuf,
        second: LocalProjectRegistration,
        second_root: PathBuf,
        second_home: PathBuf,
    }

    impl TwoComputers {
        const SESSION: &'static str = "019fb019-0000-7000-8000-00000000000a";

        fn new() -> Self {
            Self::of("claude")
        }

        fn of(provider: &'static str) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let repo = repository(&temp);
            let storage = StorageId::parse("import-after-pull-store").unwrap();
            add_local_storage(&repo, &storage, &temp.path().join("store"));
            let bundle_id = BundleId::parse("df29babc833808e68ad0efa4d01d7d6e").unwrap();
            let register = |name: &str| {
                register_local_project_with_repository(
                    &repo,
                    RegisterLocalProjectRequest {
                        display_name: name.to_string(),
                        repository_fingerprint: Some("a".repeat(64)),
                        bundle_id: Some(bundle_id.clone()),
                    },
                )
                .unwrap()
            };
            let bind = |project: &LocalProjectRegistration, root: &Path, home: &Path| {
                if provider == "codex" {
                    bind_and_link_test_replica(&repo, project, &storage, root, home);
                } else {
                    bind_and_link_claude_replica(&repo, project, &storage, root, home);
                }
            };
            let first = register("first");
            let (first_root, first_home) = (temp.path().join("proj-a"), temp.path().join("home-a"));
            bind(&first, &first_root, &first_home);
            let second = register("second");
            let (second_root, second_home) =
                (temp.path().join("proj-b"), temp.path().join("home-b"));
            bind(&second, &second_root, &second_home);
            Self {
                temp,
                repo,
                storage,
                bundle_id,
                provider,
                first,
                first_root,
                first_home,
                second,
                second_root,
                second_home,
            }
        }

        fn push(&self, project: &LocalProjectRegistration) -> ProjectOperationResult {
            let review = prepare_push_review_with_repository(
                &self.repo,
                &project.local_project_id,
                &self.storage,
            )
            .unwrap();
            ProjectSyncService::new(&self.repo)
                .push_reviewed(
                    &project.local_project_id,
                    &self.storage,
                    PushApproval::recommended(&review, &self.storage),
                )
                .unwrap()
        }

        fn pull(&self, project: &LocalProjectRegistration) -> RestoreResult {
            self.pull_reviewed(project).1
        }

        /// Apply every action the Pull review offers. Returns what it
        /// offered and what came of it.
        fn pull_reviewed(
            &self,
            project: &LocalProjectRegistration,
        ) -> (Vec<ConversationReviewAction>, RestoreResult) {
            let service = ProjectSyncService::new(&self.repo);
            let bindings = self.repo.load_bindings().unwrap();
            let plan = service
                .plan_bundle_restore(
                    &self.storage,
                    &self.bundle_id,
                    bindings.active_for(&project.local_project_id).unwrap(),
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
            (
                plan.conversation_review
                    .iter()
                    .map(|row| row.action)
                    .collect(),
                result,
            )
        }

        fn store(&self) -> PathBuf {
            self.temp.path().join("store")
        }

        /// The one session of `project`, as Refresh sees it.
        fn entry(&self, project: &LocalProjectRegistration) -> ThreadSyncEntry {
            let mut entries = get_project_thread_sync_comparison_with_repository(
                &self.repo,
                &project.local_project_id,
                &self.storage,
            )
            .unwrap()
            .entries;
            assert_eq!(entries.len(), 1, "{}: {entries:?}", self.provider);
            entries.remove(0)
        }

        /// The same, read with no saved observation.
        fn fresh_entry(&self, project: &LocalProjectRegistration) -> ThreadSyncEntry {
            super::super::super::observation_cache::observation_cache()
                .reset_and_purge_disk(self.repo.root())
                .unwrap();
            self.entry(project)
        }

        fn binding(&self, project: &LocalProjectRegistration) -> LocalConversationBinding {
            self.repo
                .load_conversation_bindings()
                .unwrap()
                .bindings
                .into_iter()
                .find(|row| row.local_project_id == project.local_project_id)
                .expect("the session has a binding")
        }

        /// The transcript under `home` whose file name holds `id`.
        fn file_of(home: &Path, id: &str) -> PathBuf {
            let mut files = Vec::new();
            jsonl_files_under(home, &mut files);
            let mut found = files
                .into_iter()
                .filter(|path| path.file_name().unwrap().to_string_lossy().contains(id))
                .collect::<Vec<_>>();
            assert_eq!(found.len(), 1, "{id}: {found:?}");
            found.remove(0)
        }

        /// The session of the first computer with two child agents, as the
        /// provider writes it. Returns the main transcript and the children.
        fn write_session_with_two_children(&self) -> (PathBuf, Vec<PathBuf>) {
            child_settings_fixture(
                &self.first_home,
                &self.first_root,
                self.provider,
                Self::SESSION,
                2,
            )
        }

        /// The children whose transcripts are stored, in the order of
        /// their keys.
        fn stored_children(&self) -> Vec<String> {
            let (_, engine) = storage_engine(&self.repo, &self.storage).unwrap();
            let mut children = engine
                .inspect(&self.bundle_id)
                .unwrap()
                .manifest
                .all_conversations()
                .flat_map(|(_, _, thread)| &thread.files)
                .filter_map(|file| match &file.role {
                    ConversationFileRole::SubagentTranscript { child_key, .. } => {
                        Some(child_key.to_string())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            children.sort();
            children
        }

        fn main_of(&self, project: &LocalProjectRegistration, home: &Path) -> PathBuf {
            let id = self.binding(project).provider_thread_id.unwrap();
            let mut files = Vec::new();
            jsonl_files_under(&home.join("projects"), &mut files);
            files
                .into_iter()
                .find(|path| path.file_name().unwrap() == format!("{id}.jsonl").as_str())
                .expect("the session has a main transcript")
        }

        /// One question, the results of the child agents `children`, and
        /// one turn with thinking, written on the first computer.
        fn write_session(&self, children: &[&str]) {
            let main = self
                .first_home
                .join("projects/source")
                .join(format!("{}.jsonl", Self::SESSION));
            let mut bytes = claude_thinking_transcript(
                Self::SESSION,
                &self.first_root,
                &[claude_row("u-1", None, "user", serde_json::json!("hello"))],
            );
            let mut parent = "u-1".to_string();
            for name in children {
                bytes.extend(child_result(Self::SESSION, &self.first_root, &parent, name));
                parent = format!("result-{name}");
                write_child(&main, Self::SESSION, &self.first_root, name);
            }
            bytes.extend(turn_with_thinking(
                Self::SESSION,
                &self.first_root,
                &parent,
                1,
            ));
            std::fs::write(&main, bytes).unwrap();
        }
    }

    fn children_of(entry: &ThreadSyncEntry) -> Vec<String> {
        entry
            .subagent_selection
            .keys
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// A row of thinking alone, a row of thinking and text, and the next
    /// question. The last row is `u-<index + 1>`.
    fn turn_with_thinking(session: &str, cwd: &Path, parent: &str, index: usize) -> Vec<u8> {
        let thought = serde_json::json!(
            {"type": "thinking", "thinking": "hidden plan ".repeat(40), "signature": "s"}
        );
        claude_thinking_transcript(
            session,
            cwd,
            &[
                claude_row(
                    &format!("t-{index}"),
                    Some(parent),
                    "assistant",
                    serde_json::json!([thought]),
                ),
                claude_row(
                    &format!("a-{index}"),
                    Some(&format!("t-{index}")),
                    "assistant",
                    serde_json::json!([thought, {"type": "text", "text": format!("answer {index}")}]),
                ),
                claude_row(
                    &format!("u-{}", index + 1),
                    Some(&format!("a-{index}")),
                    "user",
                    serde_json::json!([{"type": "text", "text": format!("question {}", index + 1)}]),
                ),
            ],
        )
    }

    /// One more question at the end of a Claude main transcript.
    fn ask_once_more(main: &Path, session: &str, cwd: &Path, uuid: &str) {
        let last: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(main)
                .unwrap()
                .lines()
                .last()
                .unwrap(),
        )
        .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(main)
            .unwrap()
            .write_all(&claude_test_transcript(
                session,
                cwd,
                &[(uuid, last["uuid"].as_str(), "user", "one more question")],
            ))
            .unwrap();
    }

    /// The result of the child agent `name` in the main transcript.
    fn child_result(session: &str, cwd: &Path, parent: &str, name: &str) -> Vec<u8> {
        let record = serde_json::json!({"type":"user","uuid":format!("result-{name}"),"parentUuid":parent,
            "sessionId":session,"cwd":cwd,"isSidechain":false,
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":format!("call-{name}"),"content":"done"}]},
            "toolUseResult":{"agentId":format!("agent-{name}"),"isAsync":true,"outputFile":"/unavailable/fixture.output"}});
        format!("{record}\n").into_bytes()
    }

    /// The transcript of the child agent `name`.
    fn write_child(main: &Path, session: &str, cwd: &Path, name: &str) {
        let agent = format!("agent-{name}");
        let path = claude_subagents::child_path(main, session, &agent).unwrap();
        let child = claude_test_transcript(
            session,
            cwd,
            &[
                ("cu1", None, "user", "child task"),
                ("ca1", Some("cu1"), "assistant", "child answer"),
            ],
        )
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut record: serde_json::Value = serde_json::from_slice(line).unwrap();
            record["agentId"] = serde_json::json!(agent);
            record["isSidechain"] = serde_json::json!(true);
            format!("{record}\n")
        })
        .collect::<String>();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, child).unwrap();
    }

    /// The first computer wrote a session and pushed it, the second pulled
    /// it. The row `a-1` names the project directory of the second
    /// computer, in text that was plain where it was written, so it is
    /// stored as it was written and its saved origin says so. Then the
    /// desktop app imports the session on the second computer, which
    /// rewrites the row. Or the row is rewritten in another way, which
    /// leaves its message as it was and which Mallard does not know. The
    /// session is continued and pushed there. Returns the two computers,
    /// the rows as they were stored first, and the result of that Push.
    fn push_of_a_pulled_session_whose_row_was_rewritten(
        in_the_conversation: bool,
        by_the_import: bool,
    ) -> (TwoComputers, Vec<serde_json::Value>, ProjectOperationResult) {
        let world = TwoComputers::new();
        let row = claude_row;
        let text = |text: &str| serde_json::json!([{"type": "text", "text": text}]);
        let notes = format!("notes are in {}/notes.txt", world.second_root.display());
        let mut rows = vec![
            row("u-1", None, "user", serde_json::json!("hello")),
            row(
                "a-1",
                Some("u-1"),
                "assistant",
                serde_json::json!([
                    {"type": "thinking", "thinking": "hidden", "signature": "s"},
                    {"type": "text", "text": notes}
                ]),
            ),
        ];
        if in_the_conversation {
            rows.push(row("u-2", Some("a-1"), "user", text("follow-up")));
        } else {
            // The first computer asked again from the first message, which
            // left `a-1` on a branch the conversation no longer follows.
            rows.push(row("u-2", Some("u-1"), "user", text("asked again")));
            rows.push(row("a-2", Some("u-2"), "assistant", text("answer")));
        }
        let last = rows.last().unwrap()["uuid"].as_str().unwrap().to_string();
        std::fs::write(
            world
                .first_home
                .join("projects/source")
                .join(format!("{}.jsonl", TwoComputers::SESSION)),
            claude_thinking_transcript(TwoComputers::SESSION, &world.first_root, &rows),
        )
        .unwrap();
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let stored = stored_rows(&world.store(), r#""uuid":"a-1""#);
        assert_eq!(
            row_of(&stored, "a-1")["message"]["content"][1]["text"],
            notes.as_str()
        );

        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{pulled:?}");
        let session = world
            .binding(&world.second)
            .provider_thread_id
            .clone()
            .unwrap();
        let main = world.main_of(&world.second, &world.second_home);
        let installed = std::fs::read(&main).unwrap();
        assert!(String::from_utf8_lossy(&installed).contains(&notes));
        let mut continued = desktop_import_bytes(&installed);
        assert!(continued.len() < installed.len());
        if !by_the_import {
            continued = continued
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .map(|line| {
                    let mut record: serde_json::Value = serde_json::from_slice(line).unwrap();
                    if record["uuid"] == "a-1" {
                        record["timestamp"] = serde_json::json!("2026-09-29T00:00:00Z");
                    }
                    format!("{record}\n")
                })
                .collect::<String>()
                .into_bytes();
        }
        continued.extend_from_slice(&claude_thinking_transcript(
            &session,
            &world.second_root,
            &[row("u-3", Some(&last), "user", text("appended"))],
        ));
        std::fs::write(&main, continued).unwrap();

        assert_eq!(
            world.entry(&world.second).state,
            ThreadSyncState::LocalAhead
        );
        let result = world.push(&world.second);
        (world, stored, result)
    }

    // The import rewrote a row that came with a Pull. The origins saved
    // for the row without its thinking are found by its digest, so the row
    // is stored as it was accepted, but for the thinking: its plain text
    // stays plain. This is so in the conversation and outside it.
    #[test]
    fn a_row_that_the_import_rewrote_is_stored_as_it_was_accepted() {
        for in_the_conversation in [true, false] {
            let (world, accepted, pushed) =
                push_of_a_pulled_session_whose_row_was_rewritten(in_the_conversation, true);
            assert!(pushed.success, "{in_the_conversation}: {pushed:?}");
            assert_eq!(failed_messages(&pushed), [""; 0], "{pushed:?}");
            let stored = stored_rows(&world.store(), r#""uuid":"u-3""#);
            let unnamed = |row: &serde_json::Value| {
                let mut row = row.clone();
                row.as_object_mut().unwrap().remove("sessionId");
                row
            };
            for row in &accepted {
                let uuid = row["uuid"].as_str().unwrap();
                assert_eq!(
                    Some(unnamed(row_of(&stored, uuid))),
                    mallard_infrastructure::providers::claude::project_claude_record(unnamed(row)),
                    "{in_the_conversation}: {uuid}"
                );
            }
            let text = row_of(&stored, "a-1")["message"]["content"][0]["text"]
                .as_str()
                .unwrap();
            assert!(
                text.starts_with("notes are in ") && !text.contains("__mallard"),
                "{text}"
            );
            for entry in [world.entry(&world.second), world.fresh_entry(&world.second)] {
                assert_eq!(entry.state, ThreadSyncState::Synced, "{entry:?}");
            }
        }
    }

    // A row that was rewritten in a way Mallard does not know finds no
    // saved origin. Outside the conversation no proof covers it: the wire
    // value at the accepted end is of the messages of the conversation. It
    // is scanned as a new record, and the text that names this computer's
    // project directory is stored as a reference to the project, although
    // it was plain text where it was written. On the computer that wrote
    // it, it then names that computer's directory. The owner accepted this
    // on 2026-09-29.
    #[test]
    fn a_row_rewritten_in_another_way_outside_the_conversation_is_scanned_as_new() {
        let (world, accepted, pushed) =
            push_of_a_pulled_session_whose_row_was_rewritten(false, false);
        assert!(pushed.success, "{pushed:?}");
        let stored = stored_rows(&world.store(), r#""uuid":"u-3""#);
        let text = row_of(&stored, "a-1")["message"]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(
            text.starts_with("notes are in /__mallard_root__/") && text.ends_with(";/notes.txt"),
            "{text}"
        );
        assert_ne!(
            text,
            row_of(&accepted, "a-1")["message"]["content"][1]["text"]
        );
        // The rows the conversation follows are stored as they were, under
        // the name the session has on this computer.
        for uuid in ["u-1", "u-2", "a-2"] {
            let unnamed = |rows: &[serde_json::Value]| {
                let mut row = row_of(rows, uuid).clone();
                row.as_object_mut().unwrap().remove("sessionId");
                row
            };
            assert_eq!(unnamed(&stored), unnamed(&accepted), "{uuid}");
        }
        assert_eq!(world.entry(&world.second).state, ThreadSyncState::Synced);
    }

    // The same row in the conversation. The messages are the accepted ones
    // in the local form, and the new wire form does not give the accepted
    // wire value, because the row would be stored as a reference. Nothing
    // is published and the session file is left as it is.
    //
    // Refresh judges the local form and never encodes, so it cannot see
    // this refusal: the session stays local progress and Push is offered
    // again.
    #[test]
    fn a_row_rewritten_in_another_way_in_the_conversation_is_refused() {
        let (world, _, refused) = push_of_a_pulled_session_whose_row_was_rewritten(true, false);
        assert!(!refused.success, "{refused:?}");
        let messages = failed_messages(&refused);
        assert_eq!(messages.len(), 1, "{refused:?}");
        assert!(
            messages[0].ends_with("path origins no longer match"),
            "{messages:?}"
        );
        assert!(stored_objects(&world.store(), r#""uuid":"u-3""#).is_empty());
        for entry in [world.entry(&world.second), world.fresh_entry(&world.second)] {
            assert_eq!(entry.state, ThreadSyncState::LocalAhead, "{entry:?}");
            assert!(entry.push_decision.selectable, "{entry:?}");
        }
    }

    // What a pulled session costs whose rows hold thinking beside other
    // content: the Pull, which saves the origins of both forms, and the
    // Push after a desktop import, which looks them up. One row in ten is
    // such a row, and one in ten holds thinking alone.
    //
    // cargo test --release --manifest-path src-tauri/Cargo.toml --lib -- \
    //     --ignored cost_sample_of_rows_with_thinking --nocapture
    #[test]
    #[ignore = "manual cost sample of 10 MiB; debug timings are not production benchmarks"]
    fn cost_sample_of_rows_with_thinking_beside_other_content() {
        let world = TwoComputers::new();
        save_child_push(&world.repo, "claude", false);
        let source = world
            .first_home
            .join("projects/source")
            .join(format!("{}.jsonl", TwoComputers::SESSION));
        let mut rows = Vec::new();
        let mut size = 0;
        while size < 10 * 1024 * 1024 {
            let index = rows.len();
            let text = format!(
                "{} {}",
                "x".repeat(1000),
                if index % 4 == 0 {
                    format!("{}/file", world.first_root.display())
                } else {
                    "ordinary text".to_string()
                }
            );
            let thinking = serde_json::json!({
                "type": "thinking", "thinking": text, "signature": "s"
            });
            let parent = index.checked_sub(1).map(|parent| format!("r-{parent}"));
            rows.push(match index % 10 {
                3 => claude_row(
                    &format!("r-{index}"),
                    parent.as_deref(),
                    "assistant",
                    serde_json::json!([thinking]),
                ),
                7 => claude_row(
                    &format!("r-{index}"),
                    parent.as_deref(),
                    "assistant",
                    serde_json::json!([thinking, {"type": "text", "text": format!("{text} {}/file", world.first_root.display())}]),
                ),
                _ => claude_row(
                    &format!("r-{index}"),
                    parent.as_deref(),
                    if index % 2 == 0 { "user" } else { "assistant" },
                    serde_json::json!([{"type": "text", "text": text}]),
                ),
            });
            size += rows[index].to_string().len() + 1;
        }
        std::fs::write(
            &source,
            claude_thinking_transcript(TwoComputers::SESSION, &world.first_root, &rows),
        )
        .unwrap();
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let timed = |label: &str, work: &mut dyn FnMut()| {
            let started = std::time::Instant::now();
            work();
            eprintln!(
                "mixed rows sample: {label}: {:.1} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        };
        timed("Pull", &mut || {
            let pulled = world.pull(&world.second);
            assert!(pulled.success, "{pulled:?}");
        });
        let binding = world.binding(&world.second);
        let session = binding.provider_thread_id.clone().unwrap();
        let main = world.main_of(&world.second, &world.second_home);
        eprintln!(
            "mixed rows sample: {} records, {} bytes, saved origins {} bytes",
            rows.len(),
            std::fs::metadata(&main).unwrap().len(),
            binding
                .path_origins_ref
                .map_or(0, |reference| reference.size_bytes)
        );
        let next = std::cell::Cell::new(rows.len());
        let append = || {
            let index = next.replace(next.get() + 1);
            std::fs::OpenOptions::new()
                .append(true)
                .open(&main)
                .unwrap()
                .write_all(&claude_thinking_transcript(
                    &session,
                    &world.second_root,
                    &[claude_row(
                        &format!("r-{index}"),
                        Some(&format!("r-{}", index - 1)),
                        "user",
                        serde_json::json!("appended"),
                    )],
                ))
                .unwrap();
        };
        let push = |label: &str| {
            timed(label, &mut || {
                let pushed = world.push(&world.second);
                assert!(pushed.success, "{label}: {pushed:?}");
                assert_eq!(failed_messages(&pushed), [""; 0], "{label}");
            });
        };
        for _ in 0..3 {
            append();
            push("Push of an appended session");
        }
        let imported = desktop_import_bytes(&std::fs::read(&main).unwrap());
        std::fs::write(&main, &imported).unwrap();
        append();
        push("Push of an imported and appended session");
        for _ in 0..3 {
            append();
            push("Push of an appended session, after the import");
        }
    }

    /// The second computer pulls a session, continues it with a child agent
    /// of its own and pushes. Then the desktop app imports the session. The
    /// import moves every byte of the file, and takes away more than was
    /// added since the Pull: the file is shorter than the one Pull wrote.
    ///
    /// The children are known by their IDs, so nothing changes for them:
    /// those that came with the Pull, the one made since, and those the
    /// Pull brought a reference of and no transcript. Returns the world
    /// after the session was continued and pushed once more.
    fn a_pulled_session_is_imported_and_pushed(
        children: &[&str],
        children_are_stored: bool,
    ) -> TwoComputers {
        let world = TwoComputers::new();
        save_child_push(&world.repo, "claude", children_are_stored);
        world.write_session(children);
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        save_child_push(&world.repo, "claude", true);

        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{pulled:?}");
        let binding = world.binding(&world.second);
        let session = binding.provider_thread_id.clone().unwrap();
        let main = world.main_of(&world.second, &world.second_home);
        let installed_length = std::fs::metadata(&main).unwrap().len();
        assert_eq!(binding.installed_end, Some(installed_length));
        let (installed, absent): (Vec<String>, Vec<String>) = if children_are_stored {
            (
                children.iter().map(|name| format!("call-{name}")).collect(),
                vec![],
            )
        } else {
            (
                vec![],
                children
                    .iter()
                    .map(|name| format!("agent-{name}"))
                    .collect(),
            )
        };
        assert_eq!(
            binding
                .installed_subagents
                .keys()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            installed
        );
        assert_eq!(
            binding
                .installed_absent_subagents
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            absent
        );

        let append = |bytes: &[u8]| {
            std::fs::OpenOptions::new()
                .append(true)
                .open(&main)
                .unwrap()
                .write_all(bytes)
                .unwrap();
        };
        append(&turn_with_thinking(&session, &world.second_root, "u-2", 2));
        append(&child_result(&session, &world.second_root, "u-3", "local"));
        write_child(&main, &session, &world.second_root, "local");
        let mut known = installed;
        known.insert(0, "call-local".to_string());
        let continued = world.entry(&world.second);
        assert_eq!(continued.state, ThreadSyncState::LocalAhead);
        assert_eq!(children_of(&continued), known);
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(world.entry(&world.second).state, ThreadSyncState::Synced);

        let before = std::fs::read(&main).unwrap();
        let imported = desktop_import_bytes(&before);
        assert!(
            (imported.len() as u64) < installed_length,
            "the import must leave the file shorter than the one Pull wrote"
        );
        std::fs::write(&main, &imported).unwrap();
        for entry in [world.entry(&world.second), world.fresh_entry(&world.second)] {
            assert_eq!(entry.state, ThreadSyncState::Synced, "{entry:?}");
            assert_eq!(children_of(&entry), known);
        }

        append(&turn_with_thinking(
            &session,
            &world.second_root,
            "result-local",
            3,
        ));
        let continued = world.entry(&world.second);
        assert_eq!(
            continued.state,
            ThreadSyncState::LocalAhead,
            "{continued:?}"
        );
        assert_eq!(children_of(&continued), known);
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(failed_messages(&pushed), [""; 0], "{pushed:?}");
        for entry in [world.entry(&world.second), world.fresh_entry(&world.second)] {
            assert_eq!(entry.state, ThreadSyncState::Synced, "{entry:?}");
            assert_eq!(children_of(&entry), known);
        }
        assert_eq!(
            world.binding(&world.second).installed_end,
            binding.installed_end
        );
        world
    }

    // The defect: Pull saved the length of the file it wrote, and Refresh
    // refused the session when that place was no longer the end of a record.
    #[test]
    fn a_desktop_import_of_a_pulled_session_is_no_edit_and_is_pushed() {
        let world = a_pulled_session_is_imported_and_pushed(&[], true);
        // The first computer takes the work of the second.
        let pulled = world.pull(&world.first);
        assert!(pulled.success, "{pulled:?}");
        assert_eq!(world.entry(&world.first).state, ThreadSyncState::Synced);
    }

    #[test]
    fn children_that_came_with_a_pull_are_known_after_a_desktop_import() {
        a_pulled_session_is_imported_and_pushed(&["one", "two"], true);
    }

    // The first computer pushed without the transcripts of its child
    // agents. The references came with the Pull and name no file on the
    // second computer. They are never taken for children made there.
    #[test]
    fn a_reference_that_came_without_its_transcript_is_no_new_child() {
        a_pulled_session_is_imported_and_pushed(&["one", "two"], false);
    }

    // What a session costs that Pull installed: the Pull, the first
    // Refresh, a Refresh of the unchanged session, and the Refresh and Push
    // of an appended one. The session holds 10 MiB and one child reference
    // in every forty rows; the children were not stored, so every reference
    // came without its transcript. `MALLARD_COST_SAMPLE_IMPORT=1` adds a
    // desktop import.
    //
    // cargo test --release --manifest-path src-tauri/Cargo.toml --lib -- \
    //     --ignored cost_sample_of_a_pulled_session --nocapture
    #[test]
    #[ignore = "manual cost sample of 10 MiB; debug timings are not production benchmarks"]
    fn cost_sample_of_a_pulled_session() {
        let world = TwoComputers::new();
        save_child_push(&world.repo, "claude", false);
        let source = world
            .first_home
            .join("projects/source")
            .join(format!("{}.jsonl", TwoComputers::SESSION));
        let mut rows = Vec::new();
        let mut size = 0;
        let mut references = 0;
        while size < 10 * 1024 * 1024 {
            let index = rows.len();
            let text = format!(
                "{} {}",
                "x".repeat(1000),
                if index % 4 == 0 {
                    format!("{}/file", world.first_root.display())
                } else {
                    "ordinary text".to_string()
                }
            );
            let thinking = serde_json::json!({
                "type": "thinking", "thinking": text, "signature": "s"
            });
            let parent = index.checked_sub(1).map(|parent| format!("r-{parent}"));
            rows.push(match index % 40 {
                3 | 13 | 23 | 33 => claude_row(
                    &format!("r-{index}"),
                    parent.as_deref(),
                    "assistant",
                    serde_json::json!([thinking]),
                ),
                20 => {
                    references += 1;
                    write_child(
                        &source,
                        TwoComputers::SESSION,
                        &world.first_root,
                        &index.to_string(),
                    );
                    serde_json::json!({"type":"user","uuid":format!("r-{index}"),"parentUuid":parent,
                        "isSidechain":false,
                        "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":format!("call-{index}"),"content":text}]},
                        "toolUseResult":{"agentId":format!("agent-{index}"),"isAsync":true,"outputFile":"/unavailable/fixture.output"}})
                }
                _ => claude_row(
                    &format!("r-{index}"),
                    parent.as_deref(),
                    if index % 2 == 0 { "user" } else { "assistant" },
                    serde_json::json!([{"type": "text", "text": text}]),
                ),
            });
            size += rows[index].to_string().len() + 1;
        }
        std::fs::write(
            &source,
            claude_thinking_transcript(TwoComputers::SESSION, &world.first_root, &rows),
        )
        .unwrap();
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        save_child_push(&world.repo, "claude", true);

        let timed = |label: &str, work: &mut dyn FnMut()| {
            let started = std::time::Instant::now();
            work();
            eprintln!(
                "pulled cost sample: {label}: {:.1} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        };
        timed("Pull", &mut || {
            let pulled = world.pull(&world.second);
            assert!(pulled.success, "{pulled:?}");
        });
        let session = world
            .binding(&world.second)
            .provider_thread_id
            .clone()
            .unwrap();
        let main = world.main_of(&world.second, &world.second_home);
        eprintln!(
            "pulled cost sample: {} records, {} bytes, {references} child references, bindings document {} bytes",
            rows.len(),
            std::fs::metadata(&main).unwrap().len(),
            std::fs::metadata(world.repo.conversation_bindings_path())
                .unwrap()
                .len()
        );
        timed("100 reads of the bindings document", &mut || {
            for _ in 0..100 {
                world.repo.load_conversation_bindings().unwrap();
            }
        });
        let refresh = |label: &str, expected: ThreadSyncState| {
            timed(label, &mut || {
                assert_eq!(world.entry(&world.second).state, expected, "{label}");
            });
        };
        let purge = || {
            super::super::super::observation_cache::observation_cache()
                .reset_and_purge_disk(world.repo.root())
                .unwrap();
        };
        refresh("first Refresh after the Pull", ThreadSyncState::Synced);
        for _ in 0..3 {
            refresh("Refresh, nothing changed", ThreadSyncState::Synced);
        }
        for _ in 0..3 {
            purge();
            refresh(
                "Refresh, nothing changed, no saved observation",
                ThreadSyncState::Synced,
            );
        }
        let next = std::cell::Cell::new(rows.len());
        let append = || {
            let index = next.replace(next.get() + 1);
            std::fs::OpenOptions::new()
                .append(true)
                .open(&main)
                .unwrap()
                .write_all(&claude_thinking_transcript(
                    &session,
                    &world.second_root,
                    &[claude_row(
                        &format!("r-{index}"),
                        Some(&format!("r-{}", index - 1)),
                        "user",
                        serde_json::json!("appended"),
                    )],
                ))
                .unwrap();
        };
        let push = |label: &str| {
            timed(label, &mut || {
                let pushed = world.push(&world.second);
                assert!(pushed.success, "{label}: {pushed:?}");
                assert_eq!(failed_messages(&pushed), [""; 0], "{label}");
            });
        };
        for _ in 0..3 {
            append();
            refresh(
                "Refresh of an appended session",
                ThreadSyncState::LocalAhead,
            );
            push("Push of an appended session");
        }
        if std::env::var("MALLARD_COST_SAMPLE_IMPORT").as_deref() != Ok("1") {
            return;
        }
        let imported = desktop_import_bytes(&std::fs::read(&main).unwrap());
        std::fs::write(&main, &imported).unwrap();
        eprintln!("pulled cost sample: imported, {} bytes", imported.len());
        refresh("first Refresh after the import", ThreadSyncState::Synced);
        for _ in 0..3 {
            refresh(
                "Refresh after the import, nothing changed",
                ThreadSyncState::Synced,
            );
        }
        append();
        refresh(
            "Refresh of an imported and appended session",
            ThreadSyncState::LocalAhead,
        );
        push("Push of an imported and appended session");
        for _ in 0..3 {
            append();
            refresh(
                "Refresh of an appended session, after the import",
                ThreadSyncState::LocalAhead,
            );
            push("Push of an appended session, after the import");
        }
    }

    // Keep both keeps the edited session as a copy of its own and installs
    // the stored one under the session's name. The copy has the origins
    // that were saved before the edit, and the edit is in its records. Its
    // first Push publishes it under a new cloud identity, so nothing stored
    // is contradicted by it. From then on it is a session like any other.
    fn the_copy_that_keep_both_preserved_is_pushed(provider: &'static str) {
        let fixture = Fixture::long(provider);
        fixture.publish_and_warm();
        let tracked = fixture.binding().cloud_thread_id.unwrap();
        fixture.edit_with_growth_is_refused_once("hello", "world", 3);
        assert_eq!(fixture.pull(), [ConversationReviewAction::KeepBoth]);
        let kept = std::fs::read(&fixture.path).unwrap();
        let copy = fixture.binding();
        assert!(copy.preserved.is_some() && copy.cloud_thread_id.is_none());
        assert!(fixture.checkpoint().accepted.is_none());
        assert!(!fixture.saved_origins_hold());
        assert_eq!(
            Fixture::sorted_states(&fixture.refresh()),
            ["LocalOnly", "Synced", "Synced"],
            "{provider}"
        );

        let pushed = fixture.push();
        assert!(pushed.success, "{provider}: {pushed:?}");
        assert_eq!(failed_messages(&pushed), [""; 0], "{provider}: {pushed:?}");
        let published = fixture.binding();
        assert!(published.preserved.is_some(), "{provider}");
        assert!(
            published
                .cloud_thread_id
                .as_ref()
                .is_some_and(|minted| *minted != tracked),
            "{provider}: {published:?}"
        );
        let bindings = fixture.repo.load_conversation_bindings().unwrap();
        assert_eq!(
            bindings
                .bindings
                .iter()
                .filter(|row| row.cloud_thread_id.as_ref() == Some(&tracked))
                .count(),
            1,
            "{provider}: the session that was installed keeps its mapping"
        );
        assert_eq!(std::fs::read(&fixture.path).unwrap(), kept, "{provider}");
        let synced = fixture.refresh();
        assert_eq!(
            Fixture::sorted_states(&synced),
            ["Synced", "Synced", "Synced"],
            "{provider}"
        );
        fixture.assert_matches_fresh_observation(&synced);

        // Continued, it is pushed by the first encoding.
        assert!(fixture.saved_origins_hold(), "{provider}");
        fixture.append_turn(4);
        assert!(fixture.saved_origins_hold(), "{provider}");
        assert_eq!(
            fixture.entry(&fixture.refresh(), fixture.active).state,
            ThreadSyncState::LocalAhead,
            "{provider}"
        );
        let pushed = fixture.push();
        assert!(pushed.success, "{provider}: {pushed:?}");
        assert_eq!(
            fixture.entry(&fixture.refresh(), fixture.active).state,
            ThreadSyncState::Synced,
            "{provider}"
        );

        // It has an accepted version now. An edit of it is an edit.
        fixture.edit_with_growth_is_refused_once("world", "globe", 5);
    }

    // What permits the second encoding is the mark Keep both left, not the
    // absence of a stored version. The same copy without the mark is a
    // local session whose records no longer match their origins.
    fn a_local_session_that_no_keep_both_preserved_stays_refused(provider: &'static str) {
        let fixture = Fixture::long(provider);
        fixture.publish_and_warm();
        fixture.edit_with_growth_is_refused_once("hello", "world", 3);
        assert_eq!(fixture.pull(), [ConversationReviewAction::KeepBoth]);
        fixture
            .repo
            .mutate_conversation_bindings(|document| {
                let row = document
                    .bindings
                    .iter_mut()
                    .find(|row| row.provider_thread_id.as_deref() == Some(fixture.active))
                    .unwrap();
                row.preserved = None;
                row.revision += 1;
                Ok(())
            })
            .unwrap();
        let origins = fixture.binding().path_origins_ref;

        let refused = fixture.push();
        let messages = failed_messages(&refused);
        assert_eq!(messages.len(), 1, "{provider}: {refused:?}");
        assert!(
            messages[0].ends_with("path origins no longer match"),
            "{provider}: {messages:?}"
        );
        let binding = fixture.binding();
        assert_eq!(binding.path_origins_ref, origins, "{provider}");
        assert!(binding.cloud_thread_id.is_none(), "{provider}");
        assert!(fixture.checkpoint().accepted.is_none(), "{provider}");
    }

    // An edit that Refresh finds by itself, with no refused Push before
    // it. Keep both then installs the stored session under the name the
    // edited one had. What Refresh had concluded of the edited file must
    // not answer for the installed one.
    #[test]
    fn keep_both_leaves_no_observation_of_the_file_it_moved() {
        let fixture = Fixture::new("claude");
        fixture.publish_and_warm();
        let edited = String::from_utf8(std::fs::read(&fixture.path).unwrap())
            .unwrap()
            .replacen("hello", "hello, edited", 1);
        std::fs::write(&fixture.path, edited).unwrap();
        fixture.append_turn(3);
        let found = fixture.refresh();
        assert_eq!(
            fixture.entry(&found, fixture.active).state,
            ThreadSyncState::Diverged
        );
        let generation = fixture.checkpoint().generation;

        assert_eq!(fixture.pull(), [ConversationReviewAction::KeepBoth]);
        let installed = fixture.refresh();
        assert_eq!(
            Fixture::sorted_states(&installed),
            ["LocalOnly", "Synced", "Synced"]
        );
        fixture.assert_matches_fresh_observation(&installed);
        let config = fixture.repo.load_config().unwrap();
        assert!(
            config
                .links
                .iter()
                .flat_map(|link| link.conversation_checkpoints.values())
                .flat_map(|threads| threads.values())
                .filter(|checkpoint| checkpoint.accepted.is_some())
                .all(|checkpoint| checkpoint.generation > generation
                    || matches!(
                        &checkpoint.physical,
                        PhysicalMappingState::Ready { root_mapping, .. }
                            if root_mapping.provider_thread_id == fixture.quiet
                    )),
            "the installed session continues the numbering of the one it replaced"
        );
    }

    // A child transcript has a cursor of its own, kept with the observation
    // of its parent. The same rule holds for it.
    fn refresh_reads_in_full_a_long_child_that_changed_without_growing(provider: &'static str) {
        for mutation in ["rewrite", "replace"] {
            let mut fixture = Fixture::new(provider);
            let child = install_child(&mut fixture);
            while std::fs::metadata(&child).unwrap().len() < 3 * jsonl_tail_io::PROBE_WINDOW_BYTES {
                let turn = std::fs::read(&child)
                    .unwrap()
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count();
                if provider == "codex" {
                    append_owned_message(&child, &format!("child-turn-{turn}"));
                } else {
                    append_claude_owned_child(
                        &child,
                        fixture.active,
                        "agent0",
                        &format!("child-turn-{turn}"),
                        &if turn == 2 {
                            "ca1".to_string()
                        } else {
                            format!("child-turn-{}", turn - 1)
                        },
                    );
                }
            }
            fixture.publish_and_warm();
            let parent = std::fs::read(&fixture.path).unwrap();
            let (word, other) = if provider == "codex" {
                ("test owned child", "test owned chilD")
            } else {
                ("child task", "child tasK")
            };
            let changed = Fixture::edited_before_the_probe_window(&child, word, other);
            let modified = std::fs::metadata(&child).unwrap().modified().unwrap();
            if mutation == "rewrite" {
                std::fs::write(&child, changed).unwrap();
            } else {
                let replacement = fixture.temp.path().join("replacement.jsonl");
                std::fs::write(&replacement, changed).unwrap();
                std::fs::rename(replacement, &child).unwrap();
            }
            #[cfg(unix)]
            Fixture::set_modified(&child, modified);
            let (current, work) = fixture.refresh_counting(&child);
            assert_eq!(
                work.probe_bytes_read, 0,
                "{provider} {mutation}: the child is not read through its cursor"
            );
            assert_eq!(
                fixture.entry(&current, fixture.active).state,
                ThreadSyncState::Diverged,
                "{provider} {mutation}"
            );
            fixture.assert_changed(&current);
            assert_eq!(std::fs::read(&fixture.path).unwrap(), parent);
        }
    }

    fn refresh_discovers_creation_deletion_and_a_replaced_session_root_without_events(
        provider: &'static str,
    ) {
        let fixture = Fixture::new(provider);
        fixture.publish_and_warm();
        let new_id = "019fb019-0000-7000-8000-000000000003";
        let new_path = fixture.path_for(new_id);
        std::fs::write(&new_path, fixture.transcript(new_id, 1)).unwrap();
        let created = fixture.refresh();
        assert_eq!(
            fixture.entry(&created, new_id).state,
            ThreadSyncState::LocalOnly
        );
        fixture.assert_matches_fresh_observation(&created);

        std::fs::remove_file(&new_path).unwrap();
        assert!(!fixture
            .views()
            .iter()
            .any(|view| view.provider_thread_id == new_id));
        assert_eq!(fixture.refresh().entries.len(), 2);

        let sessions = fixture.home.join(if provider == "codex" {
            "sessions"
        } else {
            "projects"
        });
        let parked = fixture.temp.path().join("old-session-root");
        std::fs::rename(&sessions, &parked).unwrap();
        std::fs::create_dir_all(fixture.path.parent().unwrap()).unwrap();
        std::fs::write(&fixture.path, fixture.transcript(fixture.active, 3)).unwrap();
        let current = fixture.refresh();
        assert_eq!(
            fixture.entry(&current, fixture.active).state,
            ThreadSyncState::LocalAhead
        );
        assert!(!fixture.entry(&current, fixture.quiet).local_present);
        fixture.assert_matches_fresh_observation(&current);
    }

    fn refresh_tracks_rename_archive_move_and_new_codex_physical_rollout(provider: &'static str) {
        let fixture = Fixture::new(provider);
        fixture.publish_and_warm();
        let renamed = if provider == "codex" {
            fixture
                .home
                .join("sessions/2026/09/29")
                .join(fixture.path.file_name().unwrap())
        } else {
            fixture
                .home
                .join("projects/renamed")
                .join(fixture.path.file_name().unwrap())
        };
        std::fs::create_dir_all(renamed.parent().unwrap()).unwrap();
        std::fs::rename(&fixture.path, &renamed).unwrap();
        let moved = fixture.refresh();
        assert_eq!(
            fixture.entry(&moved, fixture.active).state,
            ThreadSyncState::Synced
        );
        assert_eq!(
            fixture
                .views()
                .iter()
                .find(|view| view.provider_thread_id == fixture.active)
                .unwrap()
                .source_path,
            renamed.canonicalize().unwrap()
        );
        fixture.assert_matches_fresh_observation(&moved);
        if provider == "codex" {
            let archived = fixture
                .home
                .join("archived_sessions")
                .join(renamed.file_name().unwrap());
            std::fs::create_dir_all(archived.parent().unwrap()).unwrap();
            std::fs::rename(&renamed, &archived).unwrap();
            assert_eq!(
                fixture.entry(&fixture.refresh(), fixture.active).state,
                ThreadSyncState::Synced
            );
            let replacement = fixture.home.join("sessions/2026/09/30").join(format!(
                "rollout-2026-09-30T00-00-00-{}_resume.jsonl",
                fixture.active
            ));
            std::fs::create_dir_all(replacement.parent().unwrap()).unwrap();
            std::fs::write(&replacement, fixture.transcript(fixture.active, 3)).unwrap();
            let current = fixture.refresh();
            assert_eq!(
                fixture
                    .views()
                    .iter()
                    .find(|view| view.provider_thread_id == fixture.active)
                    .unwrap()
                    .source_path,
                replacement.canonicalize().unwrap()
            );
            assert_eq!(
                fixture.entry(&current, fixture.active).state,
                ThreadSyncState::LocalAhead
            );
            fixture.assert_changed(&current);
        }
    }

    fn refresh_reconciles_batched_changes_and_late_writer_closes(provider: &'static str) {
        let fixture = Fixture::new(provider);
        fixture.publish_and_warm();
        let mut first = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.path)
            .unwrap();
        let third = fixture.transcript(fixture.active, 3);
        first
            .write_all(&third[fixture.transcript(fixture.active, 2).len()..])
            .unwrap();
        first.flush().unwrap();
        let mut second = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.path)
            .unwrap();
        let fourth = fixture.transcript(fixture.active, 4);
        second.write_all(&fourth[third.len()..]).unwrap();
        second.flush().unwrap();
        let transient = fixture.path_for("019fb019-0000-7000-8000-000000000004");
        std::fs::write(
            &transient,
            fixture.transcript("019fb019-0000-7000-8000-000000000004", 1),
        )
        .unwrap();
        std::fs::remove_file(transient).unwrap();
        let current = fixture.refresh();
        fixture.assert_changed(&current);
        drop(second);
        assert_eq!(current.entries, fixture.refresh().entries);
        drop(first);
        assert_eq!(current.entries, fixture.refresh().entries);
    }

    fn install_child(fixture: &mut Fixture) -> PathBuf {
        let child_id = "019fb019-0000-7000-8000-000000000005";
        if fixture.provider == "codex" {
            std::fs::remove_file(&fixture.path).unwrap();
            fixture.path = write_owned_rollout(
                &fixture.home,
                &fixture.root,
                fixture.active,
                None,
                &[("spawn-child", child_id)],
            );
            write_owned_rollout(
                &fixture.home,
                &fixture.root,
                child_id,
                Some(fixture.active),
                &[],
            )
        } else {
            let (main, children) =
                write_claude_owned_fixture(&fixture.home, &fixture.root, fixture.active, 1);
            fixture.path = main;
            children[0].clone()
        }
    }

    fn refresh_recomputes_changed_children_and_recovers_a_missing_child(provider: &'static str) {
        let mut fixture = Fixture::new(provider);
        let child = install_child(&mut fixture);
        fixture.publish_and_warm();
        let parent_before = std::fs::read(&fixture.path).unwrap();
        if provider == "codex" {
            append_owned_message(&child, "child-follow-up");
        } else {
            append_claude_owned_child(&child, fixture.active, "agent0", "cu2", "ca1");
        }
        let changed = fixture.refresh();
        assert_eq!(
            fixture.entry(&changed, fixture.active).state,
            ThreadSyncState::LocalAhead
        );
        fixture.assert_changed(&changed);
        assert_eq!(std::fs::read(&fixture.path).unwrap(), parent_before);
        let parked = fixture.temp.path().join("parked-child");
        std::fs::rename(&child, &parked).unwrap();
        let missing = fixture.refresh();
        fixture.assert_incomplete(&missing);
        std::fs::rename(&parked, &child).unwrap();
        let restored = fixture.refresh();
        assert_eq!(
            fixture.entry(&restored, fixture.active).state,
            ThreadSyncState::LocalAhead
        );
        assert_eq!(
            fixture
                .entry(&restored, fixture.active)
                .subagent_selection
                .keys
                .len(),
            1
        );
        fixture.assert_changed(&restored);
    }

    fn refresh_finds_a_child_that_was_missing_when_membership_was_cached(provider: &'static str) {
        let mut fixture = Fixture::new(provider);
        fixture.publish_and_warm();
        let child = install_child(&mut fixture);
        let parked = fixture.temp.path().join("not-yet-created-child");
        std::fs::rename(&child, &parked).unwrap();
        let parent_stamp = jsonl_tail_io::SourceStamp::read(&fixture.path).unwrap();
        for _ in 0..2 {
            let missing = fixture.refresh();
            assert!(fixture
                .entry(&missing, fixture.active)
                .subagent_selection
                .keys
                .is_empty());
        }
        std::fs::rename(&parked, &child).unwrap();
        let appeared = fixture.refresh();
        assert_eq!(
            fixture
                .entry(&appeared, fixture.active)
                .subagent_selection
                .keys
                .len(),
            1
        );
        assert_eq!(
            jsonl_tail_io::SourceStamp::read(&fixture.path).unwrap(),
            parent_stamp
        );
        fixture.assert_changed(&appeared);
    }

    fn refresh_never_keeps_green_after_a_size_limit_violation(provider: &'static str) {
        let fixture = Fixture::new(provider);
        fixture.publish_and_warm();
        let writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.path)
            .unwrap();
        writer.set_len(conversation_file_limit_bytes() + 1).unwrap();
        fixture.assert_incomplete(&fixture.refresh());
        drop(writer);
    }

    #[cfg(unix)]
    fn refresh_does_not_reuse_green_when_a_source_becomes_unreadable(provider: &'static str) {
        use std::os::unix::fs::PermissionsExt;
        // A privileged process bypasses this fixture's permission denial.
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("permission-denial coverage requires an unprivileged test process");
            return;
        }
        let fixture = Fixture::new(provider);
        fixture.publish_and_warm();
        // Mode bits also change ctime, so the warmed scan and observation
        // must reach the read error.
        let permissions = std::fs::metadata(&fixture.path).unwrap().permissions();
        std::fs::set_permissions(&fixture.path, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(
            std::fs::read(&fixture.path).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let current = fixture.refresh();
        std::fs::set_permissions(&fixture.path, permissions).unwrap();
        let entry = fixture.entry(&current, fixture.active);
        assert!(
            matches!(
                entry.state,
                ThreadSyncState::Unknown | ThreadSyncState::Unavailable
            ),
            "read failure must produce incomplete evidence: {entry:?}"
        );
        assert!(!entry.push_decision.selectable);
        assert_eq!(
            fixture.entry(&current, fixture.quiet).state,
            ThreadSyncState::Synced
        );
    }

    // The schema number of the observation cache does not change with the
    // shape of a record. A saved observation whose child references have
    // the earlier shape, a key and a place in the file, does not parse. The
    // file starts empty and the session is read again.
    #[test]
    fn a_saved_observation_of_the_earlier_shape_is_read_again() {
        let mut fixture = Fixture::new("claude");
        install_child(&mut fixture);
        fixture.publish_and_warm();
        let forget = || super::super::super::observation_cache::observation_cache().reset();
        forget();
        let (saved, work) = fixture.refresh_counting(&fixture.path);
        assert_eq!(
            work.transcript_bytes_read, 0,
            "the saved observation serves"
        );
        assert_eq!(
            fixture.entry(&saved, fixture.active).state,
            ThreadSyncState::Synced
        );

        let cache = fixture.repo.root().join("observation_cache.json");
        let mut document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
        let mut references = 0;
        for entry in document["entries"].as_array_mut().unwrap() {
            let links = entry
                .pointer_mut("/observation/subagents/Claude/main/links")
                .and_then(serde_json::Value::as_object_mut);
            for (_, link) in links.into_iter().flatten() {
                *link = serde_json::json!({"key": link.clone(), "offset": 0});
                references += 1;
            }
        }
        assert_eq!(references, 1);
        std::fs::write(&cache, serde_json::to_vec(&document).unwrap()).unwrap();
        forget();
        let (current, work) = fixture.refresh_counting(&fixture.path);
        assert!(work.transcript_bytes_read > 0, "the session is read again");
        assert_eq!(current.entries, saved.entries);
    }

    /// The first computer makes a session with two child agents and pushes
    /// it with their transcripts. The second computer pulls it, continues
    /// the main transcript and one child, and pushes. Then the first
    /// computer pulls.
    ///
    /// On the first computer the children are files that the provider
    /// wrote there. The Pull that updates the session takes them as the
    /// children they are: it replaces the one that was continued, under the
    /// state it was reviewed in.
    fn the_computer_that_made_a_session_pulls_its_children(provider: &'static str) {
        let world = TwoComputers::of(provider);
        save_child_push(&world.repo, provider, true);
        let (first_main, first_children) = world.write_session_with_two_children();
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{provider}: {pushed:?}");
        let made = world.entry(&world.first);
        assert_eq!(made.state, ThreadSyncState::Synced, "{provider}");
        let children = children_of(&made);
        assert_eq!(children.len(), 2, "{provider}: {children:?}");
        assert!(world.binding(&world.first).installed_subagents.is_empty());

        let (offered, pulled) = world.pull_reviewed(&world.second);
        assert_eq!(
            offered,
            [ConversationReviewAction::CreateLocal],
            "{provider}"
        );
        assert!(pulled.success, "{provider}: {pulled:?}");
        let second = world.binding(&world.second);
        let session = second.provider_thread_id.clone().unwrap();
        let main = TwoComputers::file_of(&world.second_home, &session);
        let child = second.installed_subagents.values().next().unwrap();
        if provider == "codex" {
            append_owned_message(&main, "continued-on-the-second");
            append_owned_message(
                &TwoComputers::file_of(&world.second_home, &child.provider_thread_id),
                "child-continued-on-the-second",
            );
        } else {
            ask_once_more(
                &main,
                &session,
                &world.second_root,
                "continued-on-the-second",
            );
            append_claude_owned_child(
                &claude_subagents::child_path(&main, &session, &child.provider_thread_id).unwrap(),
                &session,
                &child.provider_thread_id,
                "child-continued-on-the-second",
                "ca1",
            );
        }
        assert_eq!(
            world.entry(&world.second).state,
            ThreadSyncState::LocalAhead,
            "{provider}"
        );
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{provider}: {pushed:?}");

        let before = first_children
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            world.entry(&world.first).state,
            ThreadSyncState::StorageAhead,
            "{provider}"
        );
        let (offered, pulled) = world.pull_reviewed(&world.first);
        assert_eq!(
            offered,
            [ConversationReviewAction::MaterializeCloud],
            "{provider}"
        );
        assert!(
            pulled.success,
            "{provider}: {} {:?}",
            pulled.message, pulled.failed_actions
        );

        let updated = world.entry(&world.first);
        assert_eq!(
            updated.state,
            ThreadSyncState::Synced,
            "{provider}: {updated:?}"
        );
        assert_eq!(children_of(&updated), children, "{provider}");
        let first = world.binding(&world.first);
        assert_eq!(
            first
                .installed_subagents
                .keys()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            children,
            "{provider}"
        );
        if provider == "claude" {
            // Claude is updated in place: the session keeps its name, and
            // so do its children.
            assert_eq!(
                first.provider_thread_id.as_deref(),
                Some(TwoComputers::SESSION)
            );
            let after = first_children
                .iter()
                .map(|path| std::fs::read(path).unwrap())
                .collect::<Vec<_>>();
            let continued = |bytes: &[u8]| {
                String::from_utf8_lossy(bytes).contains("child-continued-on-the-second")
            };
            assert_eq!(
                after.iter().filter(|bytes| continued(bytes)).count(),
                1,
                "one child was continued on the second computer"
            );
            assert_eq!(
                before
                    .iter()
                    .zip(&after)
                    .filter(|(old, new)| old != new)
                    .count(),
                1,
                "the other child is what it was"
            );
            assert!(std::fs::read_to_string(&first_main)
                .unwrap()
                .contains("continued-on-the-second"));
        }
    }

    /// A file lies where a child of the stored session belongs, and the
    /// main transcript of this computer does not refer to it. It is not a
    /// child of the session, and the Pull does not write over it.
    #[test]
    fn a_file_that_the_session_does_not_refer_to_is_never_replaced() {
        let world = TwoComputers::of("claude");
        save_child_push(&world.repo, "claude", true);
        let (first_main, first_children) = world.write_session_with_two_children();
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{pulled:?}");
        let second = world.binding(&world.second);
        let session = second.provider_thread_id.clone().unwrap();
        let main = TwoComputers::file_of(&world.second_home, &session);
        // The second computer makes a child of its own.
        let last: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(&main)
                .unwrap()
                .lines()
                .last()
                .unwrap(),
        )
        .unwrap();
        let record = serde_json::json!({"type":"user","uuid":"result-new","parentUuid":last["uuid"],
            "sessionId":session,"cwd":world.second_root,"isSidechain":false,
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-new","content":"done"}]},
            "toolUseResult":{"agentId":"agent-new","isAsync":true,"outputFile":"/unavailable/fixture.output"}});
        std::fs::OpenOptions::new()
            .append(true)
            .open(&main)
            .unwrap()
            .write_all(format!("{record}\n").as_bytes())
            .unwrap();
        let child = claude_subagents::child_path(&main, &session, "agent-new").unwrap();
        std::fs::write(
            &child,
            std::fs::read_to_string(
                claude_subagents::child_path(&main, &session, "agent0").unwrap(),
            )
            .unwrap()
            .replace("agent0", "agent-new"),
        )
        .unwrap();
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{pushed:?}");

        // On the first computer a file of that name is in the way.
        let stray =
            claude_subagents::child_path(&first_main, TwoComputers::SESSION, "agent-new").unwrap();
        std::fs::write(&stray, b"not a transcript of this session\n").unwrap();
        let (offered, pulled) = world.pull_reviewed(&world.first);
        assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
        assert!(!pulled.success, "{pulled:?}");
        assert_eq!(
            pulled
                .failed_actions
                .iter()
                .map(|failed| failed.message.as_str())
                .collect::<Vec<_>>(),
            ["child destination is occupied"]
        );
        assert_eq!(
            std::fs::read(&stray).unwrap(),
            b"not a transcript of this session\n"
        );
        for path in &first_children {
            assert!(path.exists());
        }
    }

    /// The first computer has pushed a session with two children, and the
    /// second has continued one of them and pushed. Returns the world, the
    /// main transcript of the first computer, the child that a Pull there
    /// will replace, and the other child.
    fn a_child_was_continued_on_the_second_computer() -> (TwoComputers, PathBuf, PathBuf, PathBuf) {
        let world = TwoComputers::of("claude");
        save_child_push(&world.repo, "claude", true);
        let (first_main, first_children) = world.write_session_with_two_children();
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{pulled:?}");
        let second = world.binding(&world.second);
        let session = second.provider_thread_id.clone().unwrap();
        let main = TwoComputers::file_of(&world.second_home, &session);
        let agent = second
            .installed_subagents
            .values()
            .next()
            .unwrap()
            .provider_thread_id
            .clone();
        append_claude_owned_child(
            &claude_subagents::child_path(&main, &session, &agent).unwrap(),
            &session,
            &agent,
            "child-continued-on-the-second",
            "ca1",
        );
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{pushed:?}");
        let (replaced, other): (Vec<_>, Vec<_>) = first_children.into_iter().partition(|path| {
            path.to_string_lossy()
                .ends_with(&format!("agent-{agent}.jsonl"))
        });
        (world, first_main, replaced[0].clone(), other[0].clone())
    }

    /// The messages of a child changed after the review: the Pull is
    /// refused and no file of the session is replaced.
    #[test]
    fn a_child_that_was_continued_after_the_review_is_not_replaced() {
        let (world, main, replaced, other) = a_child_was_continued_on_the_second_computer();
        let service = ProjectSyncService::new(&world.repo);
        let bindings = world.repo.load_bindings().unwrap();
        let plan = service
            .plan_bundle_restore(
                &world.storage,
                &world.bundle_id,
                bindings.active_for(&world.first.local_project_id).unwrap(),
            )
            .unwrap();
        append_claude_owned_child(
            &other,
            TwoComputers::SESSION,
            "agent1",
            "after-the-review",
            "ca1",
        );
        let before = [&main, &replaced, &other].map(|path| std::fs::read(path).unwrap());
        let applied = service
            .apply_bundle_restore(
                &plan,
                plan.conversation_review
                    .iter()
                    .filter_map(|row| row.action_id.clone())
                    .collect(),
            )
            .unwrap();
        assert!(!applied.success, "{applied:?}");
        assert_eq!(
            applied
                .failed_actions
                .iter()
                .map(|failed| failed.message.as_str())
                .collect::<Vec<_>>(),
            [CONVERSATION_CHANGED]
        );
        assert_eq!(
            [&main, &replaced, &other].map(|path| std::fs::read(path).unwrap()),
            before
        );
    }

    /// The file of a child changed while the Pull was at work, in bytes
    /// that change no message. The Pull replaces a child only in the state
    /// it has pinned, so it stops, whenever the change comes.
    #[test]
    fn a_child_whose_file_changed_during_the_pull_is_not_replaced() {
        for (after_download, refusal) in [
            (false, CONVERSATION_CHANGED),
            (true, "conversation provider target changed after review"),
        ] {
            let (world, main, replaced, other) = a_child_was_continued_on_the_second_computer();
            let change = {
                let replaced = replaced.clone();
                move || {
                    std::fs::OpenOptions::new()
                        .append(true)
                        .open(&replaced)
                        .unwrap()
                        .write_all(b"\n")
                        .unwrap();
                }
            };
            if after_download {
                pull_test_support::after_conversation_download(change);
            } else {
                pull_test_support::before_conversation_download(change);
            }
            let before = [&main, &replaced, &other].map(|path| std::fs::read(path).unwrap());
            let (offered, applied) = world.pull_reviewed(&world.first);
            assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
            assert!(!applied.success, "{after_download}: {applied:?}");
            let messages = applied
                .failed_actions
                .iter()
                .map(|failed| failed.message.lines().next().unwrap_or_default())
                .collect::<Vec<_>>();
            assert_eq!(messages, [refusal], "{after_download}");
            let mut expected = before.clone();
            expected[1].push(b'\n');
            assert_eq!(
                [&main, &replaced, &other].map(|path| std::fs::read(path).unwrap()),
                expected,
                "{after_download}"
            );
        }
    }

    // What a Pull costs that updates a session with children, in place. A
    // release build gives the figures that are worth comparing:
    //
    // cargo test --release --manifest-path src-tauri/Cargo.toml --lib -- \
    //     --ignored pull_cost_sample --nocapture
    #[test]
    #[ignore = "manual cost sample of 8 children of 1 MiB; debug timings are not production benchmarks"]
    fn pull_cost_sample_of_a_session_with_children() {
        for children in [0, 8] {
            let world = TwoComputers::of("claude");
            save_child_push(&world.repo, "claude", true);
            let main = world
                .first_home
                .join("projects/source")
                .join(format!("{}.jsonl", TwoComputers::SESSION));
            let filler = "x".repeat(1000);
            let mut bytes = claude_test_transcript(
                TwoComputers::SESSION,
                &world.first_root,
                &[("u1", None, "user", "launch agents")],
            );
            let mut previous = "u1".to_string();
            for child in 0..children {
                let agent = format!("agent{child}");
                let record = serde_json::json!({"type":"user","uuid":format!("result{child}"),"parentUuid":previous,
                    "sessionId":TwoComputers::SESSION,"cwd":world.first_root,"isSidechain":false,
                    "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":format!("call{child}"),"content":"done"}]},
                    "toolUseResult":{"agentId":agent,"isAsync":true,"outputFile":"/unavailable/fixture.output"}});
                bytes.extend_from_slice(format!("{record}\n").as_bytes());
                previous = format!("result{child}");
                let path =
                    claude_subagents::child_path(&main, TwoComputers::SESSION, &agent).unwrap();
                let mut transcript = String::new();
                let mut parent: Option<String> = None;
                while transcript.len() < 1024 * 1024 {
                    let uuid = format!("c{child}-{}", transcript.len());
                    let record = serde_json::json!({"type":"user","uuid":uuid,"parentUuid":parent,
                        "sessionId":TwoComputers::SESSION,"agentId":agent,"isSidechain":true,
                        "cwd":world.first_root,
                        "message":{"role":"user","content":format!("{filler} {}/file", world.first_root.display())}});
                    transcript.push_str(&format!("{record}\n"));
                    parent = Some(uuid);
                }
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, transcript).unwrap();
            }
            std::fs::write(&main, bytes).unwrap();
            let pushed = world.push(&world.first);
            assert!(pushed.success, "{pushed:?}");
            let timed_pull = |project: &LocalProjectRegistration, home: &Path, label: &str| {
                let service = ProjectSyncService::new(&world.repo);
                let bindings = world.repo.load_bindings().unwrap();
                // What is read and hashed of the files of this computer.
                let work = mallard_infrastructure::sync_metrics::register_root(
                    &std::fs::canonicalize(home).unwrap(),
                );
                let before_review = work.snapshot();
                let started = std::time::Instant::now();
                let plan = service
                    .plan_bundle_restore(
                        &world.storage,
                        &world.bundle_id,
                        bindings.active_for(&project.local_project_id).unwrap(),
                    )
                    .unwrap();
                let reviewed = started.elapsed();
                let before_apply = work.snapshot();
                let started = std::time::Instant::now();
                let applied = service
                    .apply_bundle_restore(
                        &plan,
                        plan.conversation_review
                            .iter()
                            .filter_map(|row| row.action_id.clone())
                            .collect(),
                    )
                    .unwrap();
                let installed = started.elapsed();
                assert!(applied.success, "{label}: {applied:?}");
                let review_work = before_apply.delta_since(&before_review);
                let apply_work = work.snapshot().delta_since(&before_apply);
                eprintln!(
                    "pull cost sample: {children} children, {label}: review {:.0} ms ({} bytes read, {} hashed), apply {:.0} ms ({} bytes read, {} hashed), {:?}",
                    reviewed.as_secs_f64() * 1000.0,
                    review_work.transcript_bytes_read,
                    review_work.bytes_hashed,
                    installed.as_secs_f64() * 1000.0,
                    apply_work.transcript_bytes_read,
                    apply_work.bytes_hashed,
                    plan.conversation_review
                        .iter()
                        .map(|row| row.action)
                        .collect::<Vec<_>>()
                );
            };
            timed_pull(
                &world.second,
                &world.second_home,
                "second computer, first Pull",
            );
            let continue_on =
                |project: &LocalProjectRegistration, home: &Path, root: &Path, turn: usize| {
                    let session = world.binding(project).provider_thread_id.unwrap();
                    ask_once_more(
                        &world.main_of(project, home),
                        &session,
                        root,
                        &format!("turn-{turn}"),
                    );
                    let pushed = world.push(project);
                    assert!(pushed.success, "{pushed:?}");
                };
            for round in 0..3 {
                continue_on(
                    &world.second,
                    &world.second_home,
                    &world.second_root,
                    2 * round,
                );
                timed_pull(
                    &world.first,
                    &world.first_home,
                    if round == 0 {
                        "computer that made the session, first Pull"
                    } else {
                        "computer that made the session, later Pull"
                    },
                );
                continue_on(
                    &world.first,
                    &world.first_home,
                    &world.first_root,
                    2 * round + 1,
                );
                timed_pull(
                    &world.second,
                    &world.second_home,
                    "second computer, later Pull",
                );
            }
        }
    }

    /// Children sync is off on both computers, as it is until the owner of
    /// a computer turns it on. The first computer makes a session with two
    /// child agents and pushes the main transcript alone. The second pulls
    /// it, continues it and pushes. Then the first computer pulls.
    ///
    /// The two children have their transcripts on the first computer, and
    /// they stay children of the session there: the session page shows
    /// them, and a Push stores them once children sync is turned on. On the
    /// second computer they are references without a transcript.
    #[test]
    fn children_made_here_stay_children_after_a_pull_of_the_main_alone() {
        let world = TwoComputers::of("claude");
        save_child_push(&world.repo, "claude", false);
        let (first_main, first_children) = world.write_session_with_two_children();
        add_the_calls_of_the_children(&first_main, &world.first_root);
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{pulled:?}");
        let second = world.binding(&world.second);
        assert_eq!(
            second.installed_absent_subagents,
            BTreeSet::from(["agent0".to_string(), "agent1".to_string()])
        );
        let session = second.provider_thread_id.clone().unwrap();
        let main = world.main_of(&world.second, &world.second_home);
        ask_once_more(
            &main,
            &session,
            &world.second_root,
            "continued-on-the-second",
        );
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{pushed:?}");

        let before = first_children
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>();
        let (offered, pulled) = world.pull_reviewed(&world.first);
        assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
        assert!(pulled.success, "{pulled:?}");
        let first = world.binding(&world.first);
        assert!(first.installed_end.is_some());
        assert!(first.installed_subagents.is_empty());
        assert!(first.installed_absent_subagents.is_empty());
        assert_eq!(
            first_children
                .iter()
                .map(|path| std::fs::read(path).unwrap())
                .collect::<Vec<_>>(),
            before
        );

        let agent_calls = |project: &LocalProjectRegistration| {
            let resource = conversation_resource_id(
                &ProviderKey::parse("claude").unwrap(),
                &world.binding(project).thread_id,
            )
            .unwrap();
            ProjectSyncService::new(&world.repo)
                .get_project_session_detail(&project.local_project_id, &resource)
                .unwrap()
                .event_counts
                .agent_calls
                .value
        };
        assert_eq!(agent_calls(&world.first), Some(2));
        assert_eq!(agent_calls(&world.second), Some(0));

        save_child_push(&world.repo, "claude", true);
        let entry = world.entry(&world.first);
        assert_eq!(children_of(&entry), ["call0", "call1"], "{entry:?}");
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(world.stored_children(), ["call0", "call1"]);
        assert_eq!(world.entry(&world.first).state, ThreadSyncState::Synced);
    }

    /// Children sync is off. The first computer makes two child agents, and
    /// the second makes one more after its Pull. Then the first computer
    /// pulls the main transcript alone.
    ///
    /// Its own two children stay children. The third is a reference that
    /// came without its transcript: the child was made on the other
    /// computer, and no file of it is here.
    #[test]
    fn a_child_made_on_another_computer_came_without_its_transcript() {
        let world = TwoComputers::of("claude");
        save_child_push(&world.repo, "claude", false);
        world.write_session_with_two_children();
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{pulled:?}");
        let session = world.binding(&world.second).provider_thread_id.unwrap();
        let main = world.main_of(&world.second, &world.second_home);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&main)
            .unwrap()
            .write_all(&child_result(
                &session,
                &world.second_root,
                "result1",
                "local",
            ))
            .unwrap();
        write_child(&main, &session, &world.second_root, "local");
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{pushed:?}");

        let (offered, pulled) = world.pull_reviewed(&world.first);
        assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
        assert!(pulled.success, "{pulled:?}");
        let first = world.binding(&world.first);
        assert!(first.installed_subagents.is_empty());
        assert_eq!(
            first.installed_absent_subagents,
            BTreeSet::from(["agent-local".to_string()])
        );
        for entry in [world.entry(&world.first), world.fresh_entry(&world.first)] {
            assert_eq!(entry.state, ThreadSyncState::Synced, "{entry:?}");
        }

        save_child_push(&world.repo, "claude", true);
        let entry = world.entry(&world.first);
        assert_eq!(children_of(&entry), ["call0", "call1"], "{entry:?}");
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(failed_messages(&pushed), [""; 0], "{pushed:?}");
        assert_eq!(world.stored_children(), ["call0", "call1"]);
        assert_eq!(world.entry(&world.first).state, ThreadSyncState::Synced);
    }

    /// What the session page shows of child agents: the count of its
    /// header, and the cards of its timeline.
    struct AgentCards {
        count: Option<u64>,
        cards: Vec<mallard_infrastructure::providers::SessionTimelineEvent>,
    }

    impl AgentCards {
        /// The page of the one session of `project`.
        fn of(world: &TwoComputers, project: &LocalProjectRegistration) -> Self {
            Self::of_session(world, project, &world.binding(project))
        }

        /// The page of one session of a project that has several.
        fn of_session(
            world: &TwoComputers,
            project: &LocalProjectRegistration,
            binding: &LocalConversationBinding,
        ) -> Self {
            let resource = conversation_resource_id(
                &ProviderKey::parse(world.provider).unwrap(),
                &binding.thread_id,
            )
            .unwrap();
            let service = ProjectSyncService::new(&world.repo);
            let detail = service
                .get_project_session_detail(&project.local_project_id, &resource)
                .unwrap();
            let mut cursor = None;
            let mut cards = Vec::new();
            loop {
                let page = service
                    .get_project_session_timeline_page(
                        &project.local_project_id,
                        &resource,
                        ProjectSessionTimelineRequest {
                            snapshot_id: detail.snapshot_id.clone(),
                            cursor,
                            limit: Some(2),
                            included_event_kinds: BTreeSet::from([
                                SessionTimelineEventKind::Prompt,
                                SessionTimelineEventKind::AgentCall,
                            ]),
                            included_tool_categories: BTreeSet::new(),
                            turn_id: None,
                        },
                    )
                    .unwrap();
                assert!(page.next_cursor.is_none() || !page.events.is_empty());
                cards.extend(page.events);
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
                assert!(cards.len() < 50, "pagination did not terminate");
            }
            cards.retain(|event| event.kind == SessionTimelineEventKind::AgentCall);
            Self {
                count: detail.event_counts.agent_calls.value,
                cards,
            }
        }

        /// The children that the cards name, in the order of the page. A
        /// card that names no child gives an empty name.
        fn named(&self) -> Vec<String> {
            self.cards
                .iter()
                .map(|event| {
                    let agent = event.agent.as_ref().unwrap();
                    agent.provider_agent_id.clone().unwrap_or_default()
                })
                .collect()
        }

        /// The transcripts that the cards lead to, in the order of the page.
        fn transcripts(&self) -> Vec<Option<String>> {
            self.cards
                .iter()
                .map(|event| event.agent.as_ref().unwrap().transcript_resource_id.clone())
                .collect()
        }
    }

    /// The main transcript of Claude with a `Task` call before each result
    /// of a child agent. The fixture holds the results alone, and the
    /// session page shows a card for each call.
    fn add_the_calls_of_the_children(main: &Path, cwd: &Path) {
        let mut records = Vec::new();
        for line in std::fs::read_to_string(main).unwrap().lines() {
            let mut record: serde_json::Value = serde_json::from_str(line).unwrap();
            if record.get("toolUseResult").is_some() {
                let call = record["message"]["content"][0]["tool_use_id"].clone();
                let task = format!("task-{}", call.as_str().unwrap());
                records.push(serde_json::json!({
                    "type":"assistant", "uuid":task, "parentUuid":record["parentUuid"],
                    "sessionId":record["sessionId"], "cwd":cwd,
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
            main,
            records
                .iter()
                .map(|record| format!("{record}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }

    fn append_record(main: &Path, record: &serde_json::Value) {
        std::fs::OpenOptions::new()
            .append(true)
            .open(main)
            .unwrap()
            .write_all(format!("{record}\n").as_bytes())
            .unwrap();
    }

    /// The call of the Claude child agent `name` at the end of the main
    /// transcript. Returns the row that a result of it follows.
    fn call_a_claude_child(main: &Path, session: &str, cwd: &Path, name: &str) -> String {
        let last: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(main)
                .unwrap()
                .lines()
                .last()
                .unwrap(),
        )
        .unwrap();
        let task = format!("task-{name}");
        append_record(
            main,
            &serde_json::json!({
                "type":"assistant", "uuid":task, "parentUuid":last["uuid"],
                "sessionId":session, "cwd":cwd,
                "message":{"role":"assistant", "content":[
                    {"type":"tool_use", "id":format!("call-{name}"), "name":"Task",
                     "input":{"description":format!("Child {name}"), "prompt":"Inspect here"}}
                ]}
            }),
        );
        task
    }

    /// A child agent that the provider made on the computer of `home`: its
    /// call and its result at the end of the main transcript, and its
    /// transcript. Returns the provider ID of the child and its transcript.
    fn make_a_child_here(
        world: &TwoComputers,
        main: &Path,
        home: &Path,
        cwd: &Path,
        session: &str,
        name: &str,
    ) -> (String, PathBuf) {
        if world.provider == "claude" {
            let task = call_a_claude_child(main, session, cwd, name);
            std::fs::OpenOptions::new()
                .append(true)
                .open(main)
                .unwrap()
                .write_all(&child_result(session, cwd, &task, name))
                .unwrap();
            write_child(main, session, cwd, name);
            let id = format!("agent-{name}");
            let path = claude_subagents::child_path(main, session, &id).unwrap();
            return (id, path);
        }
        let id = format!("019fb002-0000-7000-8000-{:012x}", 0xf00 + name.len());
        // The question of the turn that makes the child. A rollout that a
        // Pull installed holds each activity in both of its forms, inside
        // a turn, and the provider continues it the same way.
        append_owned_message(main, &format!("question-{name}"));
        let ordinal = std::fs::read(main)
            .unwrap()
            .iter()
            .filter(|byte| **byte == b'\n')
            .count();
        let turn = format!("turn-{name}");
        let payloads = [
            serde_json::json!({"type":"task_started", "turn_id":turn, "started_at":1788825780}),
            serde_json::json!({"type":"sub_agent_activity", "event_id":format!("start-{name}"),
                "kind":"started", "agent_thread_id":id, "agent_path":["worker"],
                "occurred_at_ms":43}),
            serde_json::json!({"type":"item_completed", "completed_at_ms":1788825780000u64,
                "thread_id":session, "turn_id":turn,
                "item":{"type":"SubAgentActivity", "id":format!("start-{name}"),
                    "kind":"started", "agent_thread_id":id, "agent_path":["worker"]}}),
            serde_json::json!({"type":"task_complete", "turn_id":turn, "completed_at":1788825780}),
        ];
        for (index, payload) in payloads.into_iter().enumerate() {
            append_record(
                main,
                &serde_json::json!({"ordinal":ordinal + index,
                    "timestamp":"2026-09-08T00:03:00Z", "type":"event_msg",
                    "payload":payload}),
            );
        }
        let path = write_owned_rollout(home, cwd, &id, Some(session), &[]);
        (id, path)
    }

    /// The first computer made a session with two child agents and pushed
    /// the main transcript alone, and the second pulled it. Returns the two
    /// computers and the main transcript of the second.
    fn a_session_pulled_without_its_children(provider: &'static str) -> (TwoComputers, PathBuf) {
        let world = TwoComputers::of(provider);
        save_child_push(&world.repo, provider, false);
        let (first_main, _) = world.write_session_with_two_children();
        if provider == "claude" {
            add_the_calls_of_the_children(&first_main, &world.first_root);
        }
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{provider}: {pushed:?}");
        assert!(world.stored_children().is_empty(), "{provider}");
        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{provider}: {pulled:?}");
        let session = world.binding(&world.second).provider_thread_id.unwrap();
        let main = TwoComputers::file_of(&world.second_home, &format!("{session}.jsonl"));
        (world, main)
    }

    /// A computer pulled a session without its children. A child agent that
    /// is made there afterwards has its transcript on the computer, and a
    /// Push stores it. The session page shows its card, and no card of the
    /// two children that came without a transcript.
    fn a_child_made_after_a_pull_without_children_has_its_card(provider: &'static str) {
        let (world, main) = a_session_pulled_without_its_children(provider);
        let second = world.binding(&world.second);
        let session = second.provider_thread_id.clone().unwrap();
        assert!(second.installed_subagents.is_empty(), "{provider}");
        let after_the_pull = AgentCards::of(&world, &world.second);
        assert_eq!(after_the_pull.count, Some(0), "{provider}");
        assert_eq!(after_the_pull.named(), [""; 0], "{provider}");

        let (child, _) = make_a_child_here(
            &world,
            &main,
            &world.second_home,
            &world.second_root,
            &session,
            "local",
        );
        let shown = AgentCards::of(&world, &world.second);
        assert_eq!(shown.count, Some(1), "{provider}");
        assert_eq!(shown.named(), [child.as_str()], "{provider}");
        let transcript = shown.transcripts().remove(0).expect("the card leads on");
        let page = get_project_session_detail_with_repository(
            &world.repo,
            &world.second.local_project_id,
            &transcript,
        )
        .unwrap();
        assert_eq!(page.provider_session_id, child, "{provider}");
        // The computer that made the session shows the two it made.
        assert_eq!(AgentCards::of(&world, &world.first).count, Some(2));

        save_child_push(&world.repo, provider, true);
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{provider}: {pushed:?}");
        assert_eq!(world.stored_children().len(), 1, "{provider}");
        assert_eq!(
            AgentCards::of(&world, &world.second).named(),
            [child],
            "{provider}"
        );
    }

    /// Children sync is off. The first computer makes two child agents, the
    /// second makes one more after its Pull and pushes, and the first pulls
    /// the main transcript alone. The third child came to the first
    /// computer without its transcript: the page shows the cards of the two
    /// children that are here, and no card of the third.
    #[test]
    fn a_reference_that_came_without_its_transcript_has_no_card() {
        let (world, main) = a_session_pulled_without_its_children("claude");
        let session = world.binding(&world.second).provider_thread_id.unwrap();
        make_a_child_here(
            &world,
            &main,
            &world.second_home,
            &world.second_root,
            &session,
            "local",
        );
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{pushed:?}");
        let (offered, pulled) = world.pull_reviewed(&world.first);
        assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
        assert!(pulled.success, "{pulled:?}");
        assert_eq!(
            world.binding(&world.first).installed_absent_subagents,
            BTreeSet::from(["agent-local".to_string()])
        );
        let shown = AgentCards::of(&world, &world.first);
        assert_eq!(shown.count, Some(2));
        assert_eq!(shown.named(), ["agent0", "agent1"]);
        assert!(shown.transcripts().iter().all(Option::is_some));
    }

    /// A stored version can say that children are stored and hold none: the
    /// second computer pulled the session without its children and pushes
    /// it with children sync on. A third computer pulls that version. The
    /// two references came without a transcript, and have no card.
    fn a_reference_of_a_version_stored_with_children_on_has_no_card(provider: &'static str) {
        let (world, main) = a_session_pulled_without_its_children(provider);
        let session = world.binding(&world.second).provider_thread_id.unwrap();
        if provider == "codex" {
            append_owned_message(&main, "continued-on-the-second");
        } else {
            ask_once_more(
                &main,
                &session,
                &world.second_root,
                "continued-on-the-second",
            );
        }
        save_child_push(&world.repo, provider, true);
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{provider}: {pushed:?}");
        assert!(world.stored_children().is_empty(), "{provider}");

        let third = register_local_project_with_repository(
            &world.repo,
            RegisterLocalProjectRequest {
                display_name: "third".to_string(),
                repository_fingerprint: Some("a".repeat(64)),
                bundle_id: Some(world.bundle_id.clone()),
            },
        )
        .unwrap();
        let (root, home) = (
            world.temp.path().join("proj-c"),
            world.temp.path().join("home-c"),
        );
        if provider == "codex" {
            bind_and_link_test_replica(&world.repo, &third, &world.storage, &root, &home);
        } else {
            bind_and_link_claude_replica(&world.repo, &third, &world.storage, &root, &home);
        }
        let (offered, pulled) = world.pull_reviewed(&third);
        assert_eq!(
            offered,
            [ConversationReviewAction::CreateLocal],
            "{provider}"
        );
        assert!(pulled.success, "{provider}: {pulled:?}");
        assert!(
            world.binding(&third).installed_subagents.is_empty(),
            "{provider}"
        );
        let shown = AgentCards::of(&world, &third);
        assert_eq!(shown.count, Some(0), "{provider}");
        assert_eq!(shown.named(), [""; 0], "{provider}");
    }

    /// A call that has no result names no child. It came with the Pull, and
    /// the page shows it as the computer that made it does.
    #[test]
    fn a_call_without_a_result_keeps_its_card() {
        let world = TwoComputers::of("claude");
        save_child_push(&world.repo, "claude", false);
        let (first_main, _) = world.write_session_with_two_children();
        add_the_calls_of_the_children(&first_main, &world.first_root);
        call_a_claude_child(
            &first_main,
            TwoComputers::SESSION,
            &world.first_root,
            "unanswered",
        );
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{pulled:?}");
        let native = AgentCards::of(&world, &world.first);
        assert_eq!(native.count, Some(3));
        assert_eq!(native.named(), ["agent0", "agent1", ""]);
        let shown = AgentCards::of(&world, &world.second);
        assert_eq!(shown.count, Some(1));
        assert_eq!(shown.named(), [""]);
        assert_eq!(shown.transcripts(), [None]);
    }

    /// A resumed Claude session holds the rows of the session it continues,
    /// under the ID of that session. A call among them names a child of the
    /// earlier session, which is no child of this one on any computer. The
    /// page shows it as the computer that made the session does.
    #[test]
    fn a_call_copied_from_an_earlier_session_keeps_its_card() {
        let world = TwoComputers::new();
        save_child_push(&world.repo, "claude", false);
        let main = world
            .first_home
            .join("projects/source")
            .join(format!("{}.jsonl", TwoComputers::SESSION));
        let earlier = "019fb019-0000-7000-8000-0000000000e1";
        let mut bytes = claude_test_transcript(
            earlier,
            &world.first_root,
            &[("o-1", None, "user", "the earlier session")],
        );
        for record in [
            serde_json::json!({"type":"assistant","uuid":"task-old","parentUuid":"o-1",
                "sessionId":earlier,"cwd":world.first_root,
                "message":{"role":"assistant","content":[
                    {"type":"tool_use","id":"call-old","name":"Task",
                     "input":{"description":"Earlier child","prompt":"Inspect"}}]}}),
            serde_json::json!({"type":"user","uuid":"result-old","parentUuid":"task-old",
                "sessionId":earlier,"cwd":world.first_root,"isSidechain":false,
                "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-old","content":"done"}]},
                "toolUseResult":{"agentId":"agent-old","isAsync":true,"outputFile":"/unavailable/fixture.output"}}),
        ] {
            bytes.extend_from_slice(format!("{record}\n").as_bytes());
        }
        bytes.extend(claude_test_transcript(
            TwoComputers::SESSION,
            &world.first_root,
            &[("u-1", Some("result-old"), "user", "resumed")],
        ));
        std::fs::write(&main, bytes).unwrap();
        make_a_child_here(
            &world,
            &main,
            &world.first_home,
            &world.first_root,
            TwoComputers::SESSION,
            "own",
        );
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{pulled:?}");
        assert_eq!(
            world.binding(&world.second).installed_absent_subagents,
            BTreeSet::from(["agent-own".to_string()])
        );
        let native = AgentCards::of(&world, &world.first);
        assert_eq!(native.named(), ["agent-old", "agent-own"]);
        assert_eq!(native.transcripts()[0], None);
        let shown = AgentCards::of(&world, &world.second);
        assert_eq!(shown.count, Some(1));
        assert_eq!(shown.named(), ["agent-old"]);
        assert_eq!(shown.transcripts(), [None]);
    }

    /// The Claude desktop app imports a pulled session: it rewrites the
    /// main transcript, which is shorter afterwards. A child is known by
    /// its ID, so the page shows what it showed before the import.
    #[test]
    fn the_cards_of_a_pulled_session_are_the_same_after_a_desktop_import() {
        let world = TwoComputers::new();
        save_child_push(&world.repo, "claude", false);
        world.write_session(&["a", "b"]);
        let first_main = world
            .first_home
            .join("projects/source")
            .join(format!("{}.jsonl", TwoComputers::SESSION));
        add_the_calls_of_the_children(&first_main, &world.first_root);
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{pulled:?}");
        let second = world.binding(&world.second);
        let session = second.provider_thread_id.clone().unwrap();
        let main = world.main_of(&world.second, &world.second_home);
        let (child, _) = make_a_child_here(
            &world,
            &main,
            &world.second_home,
            &world.second_root,
            &session,
            "local",
        );
        assert_eq!(
            AgentCards::of(&world, &world.second).named(),
            [child.as_str()]
        );

        let written = std::fs::read(&main).unwrap();
        let imported = desktop_import_bytes(&written);
        assert!((imported.len() as u64) < second.installed_end.unwrap());
        std::fs::write(&main, &imported).unwrap();
        let shown = AgentCards::of(&world, &world.second);
        assert_eq!(shown.count, Some(1));
        assert_eq!(shown.named(), [child]);
    }

    /// Children sync is on: the children come with their transcripts. The
    /// page of the computer that pulled shows their cards, and each card
    /// opens the transcript.
    fn children_that_came_with_their_transcripts_have_cards(provider: &'static str) {
        let world = TwoComputers::of(provider);
        save_child_push(&world.repo, provider, true);
        let (first_main, _) = world.write_session_with_two_children();
        if provider == "claude" {
            add_the_calls_of_the_children(&first_main, &world.first_root);
        }
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{provider}: {pushed:?}");
        assert_eq!(world.stored_children().len(), 2, "{provider}");
        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{provider}: {pulled:?}");
        let mut installed = world
            .binding(&world.second)
            .installed_subagents
            .into_values()
            .map(|child| child.provider_thread_id)
            .collect::<Vec<_>>();
        installed.sort();
        let shown = AgentCards::of(&world, &world.second);
        assert_eq!(shown.count, Some(2), "{provider}");
        let mut named = shown.named();
        named.sort();
        assert_eq!(named, installed, "{provider}");
        for (child, transcript) in shown.named().into_iter().zip(shown.transcripts()) {
            let page = get_project_session_detail_with_repository(
                &world.repo,
                &world.second.local_project_id,
                &transcript.expect("the card leads on"),
            )
            .unwrap();
            assert_eq!(page.provider_session_id, child, "{provider}");
        }
    }

    /// A copy of a child that an earlier Pull installed stays on the
    /// computer after a Pull of a version without children. It has no card
    /// and no page of its own: the copy belongs to the version it came with.
    #[test]
    fn a_copy_that_an_earlier_pull_installed_has_no_card_and_no_page() {
        let world = TwoComputers::of("claude");
        save_child_push(&world.repo, "claude", true);
        let (first_main, _) = world.write_session_with_two_children();
        add_the_calls_of_the_children(&first_main, &world.first_root);
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let pulled = world.pull(&world.second);
        assert!(pulled.success, "{pulled:?}");
        assert_eq!(
            AgentCards::of(&world, &world.second).named(),
            ["agent0", "agent1"]
        );

        ask_once_more(
            &first_main,
            TwoComputers::SESSION,
            &world.first_root,
            "continued-on-the-first",
        );
        save_child_push(&world.repo, "claude", false);
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        assert!(world.stored_children().is_empty());
        let (offered, pulled) = world.pull_reviewed(&world.second);
        assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
        assert!(pulled.success, "{pulled:?}");

        let second = world.binding(&world.second);
        let session = second.provider_thread_id.clone().unwrap();
        let main = world.main_of(&world.second, &world.second_home);
        for agent in ["agent0", "agent1"] {
            assert!(claude_subagents::child_path(&main, &session, agent)
                .unwrap()
                .exists());
        }
        let shown = AgentCards::of(&world, &world.second);
        assert_eq!(shown.count, Some(0));
        assert_eq!(shown.named(), [""; 0]);
        let resource =
            conversation_resource_id(&ProviderKey::parse("claude").unwrap(), &second.thread_id)
                .unwrap();
        let copy = get_project_session_detail_with_repository(
            &world.repo,
            &world.second.local_project_id,
            &format!("{resource}/agent/agent0"),
        );
        assert_eq!(
            copy.map(|page| page.title).map_err(|error| error.code),
            Err(SessionDetailErrorCode::NotFound)
        );
        let detail = ProjectSyncService::new(&world.repo)
            .get_project_session_detail(&world.second.local_project_id, &resource)
            .unwrap();
        let call = get_project_session_event_detail_with_repository(
            &world.repo,
            &world.second.local_project_id,
            resource.as_str(),
            &detail.snapshot_id,
            "claude:tool:call0",
        );
        assert_eq!(
            call.map(|_| ()).map_err(|error| error.code),
            Err(SessionDetailErrorCode::InvalidRequest)
        );
    }

    /// The bindings of the sessions of `project`.
    fn bindings_of(
        world: &TwoComputers,
        project: &LocalProjectRegistration,
    ) -> Vec<LocalConversationBinding> {
        world
            .repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .into_iter()
            .filter(|row| row.local_project_id == project.local_project_id)
            .collect()
    }

    /// The session diverged on the second computer, which made a child of
    /// its own, and the Pull keeps both. The kept copy shows the card of
    /// the child that was made here. The session that the Pull installed
    /// shows none: its two references came without a transcript.
    #[test]
    fn keep_both_leaves_each_session_its_cards() {
        let (world, main) = a_session_pulled_without_its_children("claude");
        let session = world.binding(&world.second).provider_thread_id.unwrap();
        let (child, _) = make_a_child_here(
            &world,
            &main,
            &world.second_home,
            &world.second_root,
            &session,
            "local",
        );
        ask_once_more(
            &main,
            &session,
            &world.second_root,
            "continued-on-the-second",
        );
        let first_main = world.main_of(&world.first, &world.first_home);
        ask_once_more(
            &first_main,
            TwoComputers::SESSION,
            &world.first_root,
            "continued-on-the-first",
        );
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        let (offered, pulled) = world.pull_reviewed(&world.second);
        assert_eq!(offered, [ConversationReviewAction::KeepBoth]);
        assert!(pulled.success, "{pulled:?}");

        let rows = bindings_of(&world, &world.second);
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(
                row.installed_absent_subagents,
                BTreeSet::from(["agent0".to_string(), "agent1".to_string()])
            );
            let shown = AgentCards::of_session(&world, &world.second, row);
            let kept = row.preserved.is_some();
            assert_eq!(shown.count, Some(u64::from(kept)));
            assert_eq!(
                shown.named(),
                if kept { vec![child.clone()] } else { vec![] }
            );
        }
    }

    /// A Codex update is written under a new identity. The computer that
    /// made the children pulls the main transcript alone: the rollout that
    /// the Pull wrote refers to the children, and none of them came with it.
    #[test]
    fn a_codex_update_without_children_has_no_cards() {
        let (world, main) = a_session_pulled_without_its_children("codex");
        append_owned_message(&main, "continued-on-the-second");
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(AgentCards::of(&world, &world.first).count, Some(2));
        let (offered, pulled) = world.pull_reviewed(&world.first);
        assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
        assert!(pulled.success, "{pulled:?}");
        let first = world.binding(&world.first);
        assert!(first.installed_end.is_some());
        let shown = AgentCards::of(&world, &world.first);
        assert_eq!(shown.count, Some(0));
        assert_eq!(shown.named(), [""; 0]);
    }

    /// Every session of `project`, as Refresh sees it with no saved
    /// observation.
    fn entries_of(
        world: &TwoComputers,
        project: &LocalProjectRegistration,
    ) -> Vec<ThreadSyncEntry> {
        super::super::super::observation_cache::observation_cache()
            .reset_and_purge_disk(world.repo.root())
            .unwrap();
        get_project_thread_sync_comparison_with_repository(
            &world.repo,
            &project.local_project_id,
            &world.storage,
        )
        .unwrap()
        .entries
    }

    /// The state and the children of every session of `project`, in the
    /// order of the children.
    fn sessions_of(
        world: &TwoComputers,
        project: &LocalProjectRegistration,
    ) -> Vec<(ThreadSyncState, Vec<String>)> {
        let mut sessions = entries_of(world, project)
            .iter()
            .map(|entry| (entry.state, children_of(entry)))
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| left.1.cmp(&right.1));
        sessions
    }

    fn stored_sessions(world: &TwoComputers) -> usize {
        let (_, engine) = storage_engine(&world.repo, &world.storage).unwrap();
        engine
            .inspect(&world.bundle_id)
            .unwrap()
            .manifest
            .all_conversations()
            .count()
    }

    /// Children sync is off. The first computer makes a session with two
    /// child agents and pushes the main transcript. The second pulls, makes
    /// a child of its own and pushes the main transcript. The first pulls,
    /// children sync is turned on, and the first pushes its two children.
    ///
    /// The main transcripts agree. The second computer has a child that is
    /// not stored, and the storage has two that the second computer does
    /// not have. Returns the two computers and the transcript of the child
    /// that the second computer made.
    fn each_computer_made_children_of_one_session() -> (TwoComputers, PathBuf) {
        let (world, main) = a_session_pulled_without_its_children("claude");
        let session = world.binding(&world.second).provider_thread_id.unwrap();
        let (_, child) = make_a_child_here(
            &world,
            &main,
            &world.second_home,
            &world.second_root,
            &session,
            "local",
        );
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{pushed:?}");
        assert!(world.stored_children().is_empty());
        let (offered, pulled) = world.pull_reviewed(&world.first);
        assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
        assert!(pulled.success, "{pulled:?}");
        save_child_push(&world.repo, "claude", true);
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(world.stored_children(), ["call0", "call1"]);
        (world, child)
    }

    /// A Claude session is updated in place, and the update leaves a child
    /// that Claude wrote here where it is. So the children that two
    /// computers made of one session come together: Pull installs the
    /// stored children, and the next Push stores the child of this
    /// computer. The session stays one session.
    #[test]
    fn a_pull_brings_the_children_of_two_computers_together() {
        let (world, child) = each_computer_made_children_of_one_session();
        let written = std::fs::read(&child).unwrap();
        for entry in [world.entry(&world.second), world.fresh_entry(&world.second)] {
            assert_eq!(entry.state, ThreadSyncState::StorageAhead, "{entry:?}");
            assert_eq!(children_of(&entry), ["call-local"], "{entry:?}");
            assert_eq!(
                entry.push_decision.action,
                ConversationReviewAction::RequirePull
            );
        }
        let (offered, pulled) = world.pull_reviewed(&world.second);
        assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
        assert!(pulled.success, "{pulled:?}");
        assert_eq!(std::fs::read(&child).unwrap(), written);
        let second = world.binding(&world.second);
        assert_eq!(second.installed_subagents.len(), 2);
        assert!(second.installed_absent_subagents.is_empty());
        let shown = AgentCards::of(&world, &world.second);
        assert_eq!(shown.named(), ["agent0", "agent1", "agent-local"]);
        assert!(shown.transcripts().iter().all(Option::is_some));

        for entry in [world.entry(&world.second), world.fresh_entry(&world.second)] {
            assert_eq!(entry.state, ThreadSyncState::LocalAhead, "{entry:?}");
            assert_eq!(
                children_of(&entry),
                ["call-local", "call0", "call1"],
                "{entry:?}"
            );
        }
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(failed_messages(&pushed), [""; 0], "{pushed:?}");
        assert_eq!(world.stored_children(), ["call-local", "call0", "call1"]);
        assert_eq!(stored_sessions(&world), 1);
        assert_eq!(world.entry(&world.second).state, ThreadSyncState::Synced);

        // The first computer had the third child as a reference without a
        // transcript. Now its transcript comes.
        let (offered, pulled) = world.pull_reviewed(&world.first);
        assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
        assert!(pulled.success, "{pulled:?}");
        assert!(world
            .binding(&world.first)
            .installed_absent_subagents
            .is_empty());
        assert_eq!(
            AgentCards::of(&world, &world.first).named(),
            ["agent0", "agent1", "agent-local"]
        );
        for project in [&world.first, &world.second] {
            let entries = entries_of(&world, project);
            assert_eq!(entries.len(), 1, "{entries:?}");
            assert_eq!(entries[0].state, ThreadSyncState::Synced, "{entries:?}");
        }
    }

    /// The Pull that installs the stored children replaces the main
    /// transcript. A main transcript that was continued here is not
    /// replaced: the session diverged, and the Pull keeps both.
    #[test]
    fn a_main_transcript_continued_here_beside_stored_children_keeps_both() {
        let (world, main) = a_session_pulled_without_its_children("claude");
        let session = world.binding(&world.second).provider_thread_id.unwrap();
        save_child_push(&world.repo, "claude", true);
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{pushed:?}");
        assert_eq!(world.stored_children(), ["call0", "call1"]);

        ask_once_more(
            &main,
            &session,
            &world.second_root,
            "continued-on-the-second",
        );
        let continued = std::fs::read(&main).unwrap();
        let entry = world.fresh_entry(&world.second);
        assert_eq!(entry.state, ThreadSyncState::Diverged, "{entry:?}");
        let (offered, pulled) = world.pull_reviewed(&world.second);
        assert_eq!(offered, [ConversationReviewAction::KeepBoth]);
        assert!(pulled.success, "{pulled:?}");
        assert_eq!(std::fs::read(&main).unwrap(), continued);
        assert_eq!(
            sessions_of(&world, &world.second),
            [
                (ThreadSyncState::LocalOnly, vec![]),
                (
                    ThreadSyncState::Synced,
                    vec!["call0".to_string(), "call1".to_string()]
                ),
            ]
        );
    }

    /// Children sync is off. The second computer pulls a session, makes a
    /// child of its own and pushes the main transcript. The first computer
    /// pulls, children sync is turned on, and the first makes one child
    /// more and pushes. The stored main transcript is longer than that of
    /// the second computer, which has a child that is not stored.
    ///
    /// Claude is updated in place: the Pull installs what is stored, the
    /// child that was made here stays, and the next Push stores it. A Codex
    /// update is written under a new identity, which would leave the child
    /// behind with the rollout it belongs to: the Pull keeps both.
    fn a_child_made_here_beside_a_longer_stored_session(provider: &'static str) {
        let (world, main) = a_session_pulled_without_its_children(provider);
        let session = world.binding(&world.second).provider_thread_id.unwrap();
        let (made_here, child) = make_a_child_here(
            &world,
            &main,
            &world.second_home,
            &world.second_root,
            &session,
            "local",
        );
        let written = std::fs::read(&child).unwrap();
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{provider}: {pushed:?}");
        let (offered, pulled) = world.pull_reviewed(&world.first);
        assert_eq!(
            offered,
            [ConversationReviewAction::MaterializeCloud],
            "{provider}"
        );
        assert!(pulled.success, "{provider}: {pulled:?}");
        save_child_push(&world.repo, provider, true);
        let first = world.binding(&world.first).provider_thread_id.unwrap();
        make_a_child_here(
            &world,
            &TwoComputers::file_of(&world.first_home, &format!("{first}.jsonl")),
            &world.first_home,
            &world.first_root,
            &first,
            "another",
        );
        let pushed = world.push(&world.first);
        assert!(pushed.success, "{provider}: {pushed:?}");
        assert_eq!(failed_messages(&pushed), [""; 0], "{provider}: {pushed:?}");

        let entry = world.fresh_entry(&world.second);
        assert_eq!(
            entry.push_decision.action,
            ConversationReviewAction::RequirePull,
            "{provider}"
        );
        let (offered, pulled) = world.pull_reviewed(&world.second);
        assert!(pulled.success, "{provider}: {pulled:?}");
        assert_eq!(std::fs::read(&child).unwrap(), written, "{provider}");
        let sessions = sessions_of(&world, &world.second);
        let names = |names: &[&str]| names.iter().map(ToString::to_string).collect::<Vec<_>>();
        // Each session shows the cards of the children it has: those that
        // the Pull installed, and the one that was made here.
        for row in bindings_of(&world, &world.second) {
            let mut here = row
                .installed_subagents
                .values()
                .map(|child| child.provider_thread_id.clone())
                .collect::<BTreeSet<_>>();
            if provider == "claude" || row.preserved.is_some() {
                here.insert(made_here.clone());
            }
            let shown = AgentCards::of_session(&world, &world.second, &row);
            assert_eq!(shown.count, Some(here.len() as u64), "{provider}");
            assert_eq!(
                shown.named().into_iter().collect::<BTreeSet<_>>(),
                here,
                "{provider}"
            );
            assert!(
                shown.transcripts().iter().all(Option::is_some),
                "{provider}"
            );
        }
        if provider == "claude" {
            assert_eq!(world.stored_children(), ["call-another", "call0", "call1"]);
            assert_eq!(entry.state, ThreadSyncState::StorageAhead, "{entry:?}");
            assert_eq!(offered, [ConversationReviewAction::MaterializeCloud]);
            assert_eq!(
                sessions,
                [(
                    ThreadSyncState::LocalAhead,
                    names(&["call-another", "call-local", "call0", "call1"])
                )]
            );
        } else {
            assert_eq!(world.stored_children(), ["start-another"]);
            assert_eq!(entry.state, ThreadSyncState::Diverged, "{entry:?}");
            assert_eq!(offered, [ConversationReviewAction::KeepBoth]);
            assert_eq!(
                sessions,
                [
                    (ThreadSyncState::Synced, names(&["start-another"])),
                    (ThreadSyncState::LocalOnly, names(&["start-local"])),
                ]
            );
        }
        let pushed = world.push(&world.second);
        assert!(pushed.success, "{provider}: {pushed:?}");
        assert_eq!(failed_messages(&pushed), [""; 0], "{provider}: {pushed:?}");
        let mut stored = world.stored_children();
        stored.retain(|key| key.ends_with("-local"));
        assert_eq!(stored.len(), 1, "{provider}");
        assert_eq!(
            stored_sessions(&world),
            if provider == "claude" { 1 } else { 2 },
            "{provider}"
        );
    }

    // Separate tests ensure a failure for one provider does not hide the other.
    macro_rules! provider_cases {
        ($provider:literal; $($case:ident),+ $(,)?) => {
            $(#[test] fn $case() { super::$case($provider); })+
        };
    }

    mod codex {
        provider_cases!("codex";
            refresh_sees_each_rapid_append_before_the_writer_closes,
            refresh_rejects_equal_size_rewrites_truncation_and_atomic_replacement,
            refresh_reads_in_full_a_long_transcript_that_changed_without_growing,
            refresh_reads_growth_through_the_cursor_after_a_full_read,
            an_edit_that_came_with_growth_is_refused_once_and_then_diverged,
            the_copy_that_keep_both_preserved_is_pushed,
            a_local_session_that_no_keep_both_preserved_stays_refused,
            the_computer_that_made_a_session_pulls_its_children,
            a_child_made_after_a_pull_without_children_has_its_card,
            a_reference_of_a_version_stored_with_children_on_has_no_card,
            children_that_came_with_their_transcripts_have_cards,
            a_child_made_here_beside_a_longer_stored_session,
            a_session_read_in_full_that_only_grew_is_local_progress,
            refresh_reads_in_full_a_long_child_that_changed_without_growing,
            refresh_discovers_creation_deletion_and_a_replaced_session_root_without_events,
            refresh_tracks_rename_archive_move_and_new_codex_physical_rollout,
            refresh_reconciles_batched_changes_and_late_writer_closes,
            refresh_recomputes_changed_children_and_recovers_a_missing_child,
            refresh_finds_a_child_that_was_missing_when_membership_was_cached,
            refresh_never_keeps_green_after_a_size_limit_violation,
        );
        #[cfg(unix)]
        provider_cases!("codex"; refresh_does_not_reuse_green_when_a_source_becomes_unreadable);
    }

    mod claude {
        provider_cases!("claude";
            refresh_sees_each_rapid_append_before_the_writer_closes,
            refresh_rejects_equal_size_rewrites_truncation_and_atomic_replacement,
            refresh_reads_in_full_a_long_transcript_that_changed_without_growing,
            refresh_reads_growth_through_the_cursor_after_a_full_read,
            an_edit_that_came_with_growth_is_refused_once_and_then_diverged,
            the_copy_that_keep_both_preserved_is_pushed,
            a_local_session_that_no_keep_both_preserved_stays_refused,
            the_computer_that_made_a_session_pulls_its_children,
            a_child_made_after_a_pull_without_children_has_its_card,
            a_reference_of_a_version_stored_with_children_on_has_no_card,
            children_that_came_with_their_transcripts_have_cards,
            a_child_made_here_beside_a_longer_stored_session,
            a_session_read_in_full_that_only_grew_is_local_progress,
            refresh_reads_in_full_a_long_child_that_changed_without_growing,
            refresh_discovers_creation_deletion_and_a_replaced_session_root_without_events,
            refresh_tracks_rename_archive_move_and_new_codex_physical_rollout,
            refresh_reconciles_batched_changes_and_late_writer_closes,
            refresh_recomputes_changed_children_and_recovers_a_missing_child,
            refresh_finds_a_child_that_was_missing_when_membership_was_cached,
            refresh_never_keeps_green_after_a_size_limit_violation,
        );
        #[cfg(unix)]
        provider_cases!("claude"; refresh_does_not_reuse_green_when_a_source_becomes_unreadable);
    }

    include!("session_pull_tests.rs");
    include!("stored_session_tests.rs");
}
