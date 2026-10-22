//! Live Push and Pull 5 -> 2 -> 1 retries. Only Codex writes the race
//! updates. Pull checks each source once and accepts an append after that check.

use super::*;
use mallard_domain::{AcceptedConversationVersion, PortableConversationTail};

pub(super) const EXPECTED_TURNS: usize = 24;
const VERIFY_REPLICA: usize = 2;

fn markers(stage: &str, thread: usize) -> (String, String) {
    (
        format!("LIVE_USER_ISOLATION_{stage}_{thread}"),
        format!("MALLARD_ISOLATION_{stage}_{thread}"),
    )
}

fn extend_history(history: &mut Vec<HistoryMarker>, stage: &str, thread: usize) {
    let (user, assistant) = markers(stage, thread);
    history.extend(marker_turn(&user, &assistant));
}

fn source_history(thread: usize) -> Vec<HistoryMarker> {
    let mut history = Vec::new();
    extend_history(&mut history, "SEED", thread);
    if thread >= 4 {
        extend_history(&mut history, "PUSH1", thread);
    }
    if thread == 5 {
        extend_history(&mut history, "PUSH2", thread);
    }
    history
}

fn final_history(thread: usize) -> Vec<HistoryMarker> {
    let mut history = source_history(thread);
    extend_history(&mut history, "REMOTE", thread);
    history
}

fn preserved_history(thread: usize) -> Vec<HistoryMarker> {
    let mut history = source_history(thread);
    extend_history(&mut history, "PULL1", thread);
    if thread == 5 {
        extend_history(&mut history, "PULL2", thread);
        extend_history(&mut history, "PULL3", thread);
    }
    history
}

pub(super) fn scenarios() -> Vec<AimockScenario> {
    let mut scenarios = Vec::new();
    for thread in 1..=5 {
        let mut history = Vec::new();
        let stages = if thread == 5 {
            vec!["SEED", "PUSH1", "PUSH2", "REMOTE"]
        } else if thread == 4 {
            vec!["SEED", "PUSH1", "REMOTE"]
        } else {
            vec!["SEED", "REMOTE"]
        };
        for stage in stages {
            let (user, assistant) = markers(stage, thread);
            scenarios.push(aimock_text_scenario(
                &format!("isolation_{}_{thread}", stage.to_lowercase()),
                &user,
                history.clone(),
                &assistant,
            ));
            history.extend(marker_turn(&user, &assistant));
        }
    }
    for thread in 4..=5 {
        let mut history = source_history(thread);
        for stage in if thread == 5 {
            vec!["PULL1", "PULL2", "PULL3"]
        } else {
            vec!["PULL1"]
        } {
            let (user, assistant) = markers(stage, thread);
            scenarios.push(aimock_text_scenario(
                &format!("isolation_{}_{thread}", stage.to_lowercase()),
                &user,
                history.clone(),
                &assistant,
            ));
            history.extend(marker_turn(&user, &assistant));
        }
    }
    for (index, history) in (1..=5)
        .map(final_history)
        .chain((4..=5).map(preserved_history))
        .enumerate()
    {
        scenarios.push(aimock_text_scenario(
            &format!("isolation_verify_{index}"),
            &format!("MALLARD_USER_VERIFY_{VERIFY_REPLICA}_{index}"),
            history,
            HEADLESS_REOPEN_ACK,
        ));
    }
    assert_eq!(scenarios.len(), EXPECTED_TURNS);
    scenarios
}

struct Replica {
    repository: LocalStateRepository,
    process_home: PathBuf,
    codex_home: PathBuf,
    project_root: PathBuf,
    detail: ProjectDetail,
}

impl Replica {
    fn new(root: &Path, storage: StorageConfig, choice: DraftRepositoryChoice) -> Self {
        let process_home = root.join("process-home");
        let codex_home = root.join("codex-home");
        let project_root = root.join("project");
        for path in [&process_home, &project_root] {
            std::fs::create_dir_all(path).expect("create isolation fixture directory");
        }
        create_secure_codex_home(&codex_home);
        let project_root = canonicalize(project_root).expect("canonicalize isolation project");
        let repository = redacted(
            LocalStateRepository::from_home_dir(&root.join("machine")),
            "create isolation repository",
        );
        let detail = finish_setup(
            &repository,
            &ProjectSyncService::new(&repository),
            &project_root,
            &codex_home,
            storage,
            choice,
        );
        Self {
            repository,
            process_home,
            codex_home,
            project_root,
            detail,
        }
    }

    fn service(&self) -> ProjectSyncService<'_> {
        ProjectSyncService::new(&self.repository)
    }

    fn id(&self) -> &LocalProjectId {
        &self.detail.project.local_project_id
    }

    fn binding(&self) -> &ProjectBinding {
        self.detail.binding.as_ref().expect("isolation binding")
    }

    fn row(&self, thread: &ThreadId) -> LocalConversationBinding {
        conversation_binding(&self.repository, self.id(), thread)
    }

    fn accepted(
        &self,
        storage: &StorageId,
        thread: &ThreadId,
    ) -> Option<(AcceptedConversationVersion, PortableConversationTail)> {
        let config = self
            .repository
            .load_config()
            .expect("load isolation checkpoints");
        let checkpoint = config
            .links
            .iter()
            .find(|link| &link.storage_id == storage && &link.local_project_id == self.id())?
            .conversation_checkpoints
            .get(&SyncScopeKey::provider(codex_key()))?
            .get(thread)?;
        checkpoint
            .accepted
            .clone()
            .zip(checkpoint.accepted_local_tail.clone())
    }

    fn turn(
        &self,
        provider: &LocalAimockProvider,
        turns: &mut usize,
        alias: Option<&str>,
        stage: &str,
        thread: usize,
    ) -> String {
        let (user, assistant) = markers(stage, thread);
        let before = alias.map(|alias| {
            read_bounded_file(
                &rollout_path(&self.codex_home, alias),
                MAX_CONVERSATION_FILE_BYTES,
                "isolation before turn",
            )
        });
        let result = run_codex(
            turns,
            provider,
            &self.codex_home,
            &self.process_home,
            &self.project_root,
            CodexRunAccess::ReadOnly,
            alias,
            &user,
            &assistant,
        );
        if let Some(alias) = alias {
            assert_eq!(result, alias);
        }
        let after = read_bounded_file(
            &rollout_path(&self.codex_home, &result),
            MAX_CONVERSATION_FILE_BYTES,
            "isolation after turn",
        );
        if let Some(before) = before {
            assert!(
                after.len() > before.len() && after.starts_with(&before),
                "race update did not append"
            );
        }
        assert!(
            byte_slice_contains(&after, assistant.as_bytes()),
            "Codex did not flush the race marker"
        );
        result
    }
}

fn selected_resources(
    review: &mallard_application::project_sync::service::PushReviewContext,
    aliases: &[String],
) -> BTreeSet<ResourceId> {
    let selected = review
        .thread_comparison
        .as_ref()
        .expect("isolation comparison")
        .entries
        .iter()
        .filter(|entry| {
            entry
                .provider_thread_id
                .as_ref()
                .is_some_and(|alias| aliases.contains(alias))
        })
        .map(|entry| entry.resource_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        selected.len(),
        aliases.len(),
        "selection lost an isolation thread"
    );
    selected
}

fn prepare_push(
    replica: &Replica,
    storage: &StorageId,
    aliases: &[String],
) -> (PushApproval, BTreeSet<ResourceId>) {
    let review = redacted(
        replica.service().prepare_push_review(replica.id(), storage),
        "prepare isolation Push",
    );
    assert!(matches!(
        review.destination_status,
        PushDestinationStatus::Ready { .. }
    ));
    let selected = selected_resources(&review, aliases);
    let mut approval = PushApproval::recommended(&review, storage);
    approval
        .recipe
        .entries
        .retain(|resource, _| selected.contains(resource));
    assert_eq!(
        approval
            .recipe
            .entries
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        selected
    );
    (approval, selected)
}

fn stored_threads(replica: &Replica, storage: &StorageId) -> BTreeMap<CloudThreadId, String> {
    let snapshot = redacted(
        replica
            .service()
            .inspect_bundle(storage, &replica.detail.project.bundle_id),
        "inspect isolation bundle",
    );
    snapshot
        .manifest
        .all_conversations()
        .map(|(_, id, thread)| {
            let transcript = thread
                .files
                .iter()
                .find(|file| file.role == ConversationFileRole::Transcript)
                .expect("stored transcript");
            let (_, entry) = snapshot
                .manifest
                .find_file(&transcript.logical_path)
                .expect("stored transcript entry");
            (id.clone(), entry.sha256.clone())
        })
        .collect()
}

fn head(replica: &Replica, storage: &StorageId) -> (u64, String, String) {
    let snapshot = redacted(
        replica
            .service()
            .inspect_bundle(storage, &replica.detail.project.bundle_id),
        "inspect isolation head",
    );
    (
        snapshot.head.revision,
        snapshot.head.commit_id,
        snapshot.head.manifest_sha256,
    )
}

pub(super) fn run(provider: &LocalAimockProvider) {
    eprintln!("live Codex phase: five-thread Push and Pull failure isolation");
    let root = tempfile::tempdir().expect("create failure isolation fixture");
    let storage_root = root.path().join("storage");
    std::fs::create_dir(&storage_root).expect("create isolation storage");
    let storage = local_storage(&storage_root);
    let storage_id = storage.id.clone();
    let a = Replica::new(
        &root.path().join("a"),
        storage.clone(),
        DraftRepositoryChoice::New,
    );
    let mut turns = 0;
    let aliases = (1..=5)
        .map(|thread| a.turn(provider, &mut turns, None, "SEED", thread))
        .collect::<Vec<_>>();
    let initial_review = a
        .service()
        .prepare_push_review(a.id(), &storage_id)
        .expect("review initial isolation threads");
    let threads = aliases
        .iter()
        .map(|alias| {
            let entry = initial_review
                .thread_comparison
                .as_ref()
                .expect("initial comparison")
                .entries
                .iter()
                .find(|entry| entry.provider_thread_id.as_ref() == Some(alias))
                .expect("initial thread review row");
            mallard_domain::parse_conversation_resource_id(entry.conversation_resource_id.as_str())
                .expect("reviewed local thread identity")
                .1
        })
        .collect::<Vec<_>>();
    let mut pending = (0..5).collect::<BTreeSet<_>>();

    for (round, moving) in [vec![3, 4], vec![4], vec![]].into_iter().enumerate() {
        let started = Instant::now();
        let selected_aliases = pending
            .iter()
            .map(|&index| aliases[index].clone())
            .collect::<Vec<_>>();
        let (approval, selected) = prepare_push(&a, &storage_id, &selected_aliases);
        let review = a
            .service()
            .prepare_push_review(a.id(), &storage_id)
            .expect("review moving Push threads");
        let failed_expected = selected_resources(
            &review,
            &moving
                .iter()
                .map(|&i| aliases[i].clone())
                .collect::<Vec<_>>(),
        );
        let before = stored_threads(&a, &storage_id);
        let before_head = head(&a, &storage_id);
        let accepted_before = threads
            .iter()
            .map(|id| a.accepted(&storage_id, id))
            .collect::<Vec<_>>();
        let mut barriers = 0;
        let result = redacted(
            a.service().push_reviewed_with_progress(
                a.id(),
                &storage_id,
                approval,
                &mut |_| {},
                &mut |phase, _| {
                    if phase == "build conversation actions" {
                        barriers += 1;
                        assert_eq!(barriers, 1, "Push barrier repeated");
                        for &index in &moving {
                            a.turn(
                                provider,
                                &mut turns,
                                Some(&aliases[index]),
                                &format!("PUSH{}", round + 1),
                                index + 1,
                            );
                        }
                    }
                },
            ),
            "isolation Push aborted the batch",
        );
        assert_eq!(barriers, 1, "Push barrier was not reached");
        let failed = result
            .results
            .iter()
            .filter(|row| row.state == "failed")
            .map(|row| {
                assert!(
                    row.message
                        .as_deref()
                        // The barrier appends after capture. Push now reuses
                        // captured bytes and rejects the changed source stamp.
                        .is_some_and(|message| message.contains("changed after capture")),
                    "unexpected Push failure"
                );
                row.resource_id.clone()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(failed, failed_expected, "Push failed the wrong threads");
        assert_eq!(result.success, moving.is_empty());
        assert!(failed.is_subset(&selected));
        let after = stored_threads(&a, &storage_id);
        assert_eq!(after.len(), [3, 4, 5][round]);
        assert_eq!(head(&a, &storage_id).0, before_head.0 + 1);
        for (cloud, digest) in before {
            assert_eq!(
                after.get(&cloud),
                Some(&digest),
                "retry rewrote a completed thread"
            );
        }
        for (index, id) in threads.iter().enumerate() {
            if moving.contains(&index) {
                let bindings = a
                    .repository
                    .load_conversation_bindings()
                    .expect("read failed Push binding");
                assert!(
                    bindings
                        .bindings
                        .iter()
                        .find(|row| row.local_project_id == *a.id() && &row.thread_id == id)
                        .is_none_or(|row| row.cloud_thread_id.is_none()),
                    "failed Push installed a mapping"
                );
                assert_eq!(
                    a.accepted(&storage_id, id),
                    accepted_before[index],
                    "failed Push advanced its checkpoint"
                );
            } else {
                let row = a.row(id);
                assert!(after.contains_key(
                    row.cloud_thread_id
                        .as_ref()
                        .expect("successful Push mapping")
                ));
                assert!(a.accepted(&storage_id, id).is_some());
                if !pending.contains(&index) {
                    assert_eq!(a.accepted(&storage_id, id), accepted_before[index]);
                }
            }
        }
        let next = a
            .service()
            .prepare_push_review(a.id(), &storage_id)
            .expect("review Push remainder");
        let remaining = next
            .thread_comparison
            .as_ref()
            .expect("remainder comparison")
            .entries
            .iter()
            .filter(|row| row.state != ThreadSyncState::Synced)
            .map(|row| row.resource_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(remaining, failed, "Push remainder differs from failures");
        eprintln!(
            "live Codex isolation Push {}: {} pushed, {} remaining ({:.3}s)",
            round + 1,
            pending.len() - moving.len(),
            moving.len(),
            started.elapsed().as_secs_f64()
        );
        pending = moving.into_iter().collect();
    }
    let before_noop = head(&a, &storage_id);
    assert!(push(&a.service(), a.id(), &storage_id).success);
    assert_eq!(
        head(&a, &storage_id),
        before_noop,
        "no-op Push wrote a revision"
    );
    let clouds = threads
        .iter()
        .map(|id| cloud_thread_id_of(&a.repository, a.id(), id))
        .collect::<Vec<_>>();
    for (index, cloud) in clouds.iter().enumerate() {
        let bytes = portable_conversation_rollout(
            &a.service(),
            &storage_id,
            &a.detail.project.bundle_id,
            &storage_root,
            cloud,
        );
        for item in source_history(index + 1) {
            assert!(
                byte_slice_contains(&bytes, item.marker.as_bytes()),
                "Push lost a race marker"
            );
        }
    }

    let b = Replica::new(
        &root.path().join("b"),
        storage.clone(),
        DraftRepositoryChoice::Existing {
            storage_id: storage_id.clone(),
            bundle_id: a.detail.project.bundle_id.clone(),
            display_name: a.detail.project.display_name.clone(),
            repository_fingerprint: a.detail.project.repository_fingerprint.clone(),
            mismatch_acknowledged: true,
        },
    );
    pull_and_apply(
        &b.service(),
        &storage_id,
        &a.detail.project.bundle_id,
        b.binding(),
    );
    let b_threads = clouds
        .iter()
        .map(|cloud| local_thread_of_cloud(&b.repository, b.id(), cloud))
        .collect::<Vec<_>>();
    let b_aliases = b_threads
        .iter()
        .map(|id| b.row(id).provider_thread_id.expect("local Pull alias"))
        .collect::<Vec<_>>();
    for (index, alias) in aliases.iter().enumerate() {
        a.turn(provider, &mut turns, Some(alias), "REMOTE", index + 1);
    }
    assert!(push(&a.service(), a.id(), &storage_id).success);
    let fixed_storage = storage_digest_inventory(&storage_root);
    let fixed_head = head(&a, &storage_id);
    pending = (0..5).collect();

    for (round, moving) in [vec![3, 4], vec![4], vec![]].into_iter().enumerate() {
        // Both MaterializeCloud and approved KeepBoth fail a changed
        // source. Only a new user Apply retries the failed threads.
        let started = Instant::now();
        let plan = redacted(
            b.service()
                .plan_bundle_restore(&storage_id, &a.detail.project.bundle_id, b.binding()),
            "prepare isolation Pull",
        );
        let rows = plan
            .conversation_review
            .iter()
            .filter(|row| {
                row.cloud_thread_id
                    .as_ref()
                    .is_some_and(|id| pending.iter().any(|&index| &clouds[index] == id))
            })
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), pending.len());
        let expected_action = if round == 0 {
            ConversationReviewAction::MaterializeCloud
        } else {
            ConversationReviewAction::KeepBoth
        };
        assert!(
            rows.iter().all(|row| row.action == expected_action),
            "Pull retry offered an unexpected action"
        );
        let approved = rows
            .iter()
            .map(|row| row.action_id.clone().expect("approved Pull identity"))
            .collect::<Vec<_>>();
        let failed_expected = rows
            .iter()
            .filter(|row| {
                moving
                    .iter()
                    .any(|&index| row.cloud_thread_id.as_ref() == Some(&clouds[index]))
            })
            .map(|row| row.action_id.clone().unwrap())
            .collect::<BTreeSet<_>>();
        let accepted_before = b_threads
            .iter()
            .map(|id| b.accepted(&storage_id, id))
            .collect::<Vec<_>>();
        let rows_before = b_threads.iter().map(|id| b.row(id)).collect::<Vec<_>>();
        let mut race_digests = BTreeMap::new();
        let mut barriers = 0;
        let mut source_checks = 0;
        let mut source_check_time = Duration::ZERO;
        let mut prepared_threads = 0;
        let mut preparation_work = Duration::ZERO;
        let mut install_work = Duration::ZERO;
        let mut pipeline_time = Duration::ZERO;
        let source_metrics = pending
            .iter()
            .map(|&index| {
                let path = std::fs::canonicalize(rollout_path(&b.codex_home, &b_aliases[index]))
                    .expect("canonical source for read metrics");
                (
                    index,
                    mallard_infrastructure::sync_metrics::register_root(&path),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut prepared_source_metrics = BTreeMap::new();
        let result = redacted(
            b.service().apply_bundle_restore_with_progress(
                &plan,
                approved.clone(),
                &mut |_| {},
                &mut |phase, elapsed| {
                    if phase.starts_with("prepare transcript ") {
                        prepared_threads += 1;
                        preparation_work += elapsed;
                        assert!(
                            phase.ends_with("2 semantic analyses)"),
                            "unexpected transcript analysis count: {phase}"
                        );
                    }
                    if phase == "stage and commit conversation" {
                        install_work += elapsed;
                    }
                    if phase.starts_with("prepare and install conversations (") {
                        pipeline_time = elapsed;
                    }
                    if phase == "prepare conversations" {
                        barriers += 1;
                        assert_eq!(barriers, 1, "Pull barrier repeated");
                        for &index in &moving {
                            b.turn(
                                provider,
                                &mut turns,
                                Some(&b_aliases[index]),
                                &format!("PULL{}", round + 1),
                                index + 1,
                            );
                            let bytes = read_bounded_file(
                                &rollout_path(&b.codex_home, &b_aliases[index]),
                                MAX_CONVERSATION_FILE_BYTES,
                                "local race transcript",
                            );
                            race_digests.insert(index, hex_sha256(&bytes));
                        }
                        prepared_source_metrics = pending
                            .iter()
                            .map(|&index| {
                                let size = std::fs::metadata(rollout_path(
                                    &b.codex_home,
                                    &b_aliases[index],
                                ))
                                .expect("measure checked source")
                                .len();
                                (index, (source_metrics[&index].snapshot(), size))
                            })
                            .collect();
                    }
                    if phase == "check local conversation" {
                        source_checks += 1;
                        source_check_time += elapsed;
                        if round == 2 {
                            assert_eq!(source_checks, 1, "Pull repeated the source check");
                            // This write starts after the only check. The
                            // approved KeepBoth must finish without a retry.
                            b.turn(provider, &mut turns, Some(&b_aliases[4]), "PULL3", 5);
                            let bytes = read_bounded_file(
                                &rollout_path(&b.codex_home, &b_aliases[4]),
                                MAX_CONVERSATION_FILE_BYTES,
                                "late local race transcript",
                            );
                            race_digests.insert(4, hex_sha256(&bytes));
                        }
                    }
                },
            ),
            "isolation Pull aborted the batch",
        );
        assert_eq!(barriers, 1, "Pull barrier was not reached");
        assert_eq!(
            prepared_threads,
            pending.len(),
            "each selected cloud transcript must prepare once"
        );
        assert_eq!(
            source_checks,
            pending.len(),
            "Pull must check each source once"
        );
        let mut source_check_bytes = 0;
        for (&index, metrics) in &source_metrics {
            let (before, size) = &prepared_source_metrics[&index];
            let work = metrics.snapshot().delta_since(before);
            assert_eq!(
                work.transcript_bytes_read,
                *size,
                "Pull reread source T{}",
                index + 1
            );
            assert_eq!(
                work.bytes_hashed,
                *size,
                "Pull rehashed source T{}",
                index + 1
            );
            source_check_bytes += work.transcript_bytes_read;
        }
        let failed = result
            .failed_actions
            .iter()
            .map(|failure| {
                assert!(
                    failure.message.contains("changed after review"),
                    "unexpected Pull failure"
                );
                failure.action_id.clone()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(failed, failed_expected, "Pull failed the wrong threads");
        let applied = result
            .applied_action_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            applied,
            approved
                .into_iter()
                .filter(|id| !failed.contains(id))
                .collect()
        );
        assert_eq!(result.success, moving.is_empty());
        assert_eq!(result.reviewed_base_advanced, Some(moving.is_empty()));
        assert_eq!(head(&a, &storage_id), fixed_head);
        assert_eq!(
            storage_digest_inventory(&storage_root),
            fixed_storage,
            "Pull wrote to storage"
        );
        for (index, id) in b_threads.iter().enumerate() {
            if moving.contains(&index) || !pending.contains(&index) {
                assert_eq!(
                    b.accepted(&storage_id, id),
                    accepted_before[index],
                    "unapplied thread checkpoint changed"
                );
                assert_eq!(
                    b.row(id),
                    rows_before[index],
                    "unapplied thread binding changed"
                );
            } else {
                assert_ne!(
                    b.accepted(&storage_id, id),
                    accepted_before[index],
                    "successful Pull did not advance its checkpoint"
                );
            }
            if let Some(expected) = race_digests.get(&index) {
                let bytes = read_bounded_file(
                    &rollout_path(&b.codex_home, &b_aliases[index]),
                    MAX_CONVERSATION_FILE_BYTES,
                    "failed Pull local transcript",
                );
                assert_eq!(
                    &hex_sha256(&bytes),
                    expected,
                    "Pull changed the continuing local transcript"
                );
            }
        }
        assert_eq!(
            result.preserved_threads.len(),
            if round == 0 { 0 } else { 1 }
        );
        let next = b
            .service()
            .plan_bundle_restore(&storage_id, &a.detail.project.bundle_id, b.binding())
            .expect("review Pull remainder");
        let remaining = next
            .conversation_review
            .iter()
            .filter(|row| row.action_id.is_some())
            .filter_map(|row| row.cloud_thread_id.as_ref())
            .filter(|id| clouds.contains(id))
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            remaining,
            moving.iter().map(|&index| clouds[index].clone()).collect(),
            "Pull remainder differs from failures"
        );
        eprintln!(
            "live Codex isolation Pull {}: {} restored, {} remaining ({:.3}s); \
             {} source checks, {} bytes, {:.3}ms",
            round + 1,
            applied.len(),
            moving.len(),
            started.elapsed().as_secs_f64(),
            source_checks,
            source_check_bytes,
            source_check_time.as_secs_f64() * 1_000.0,
        );
        eprintln!(
            "live Codex isolation Pull {} preparation: {} threads, {:.3}ms worker time, {:.3}ms stage/commit time, {:.3}ms pipeline wall time",
            round + 1, prepared_threads, preparation_work.as_secs_f64() * 1_000.0,
            install_work.as_secs_f64() * 1_000.0, pipeline_time.as_secs_f64() * 1_000.0,
        );
        pending = moving.into_iter().collect();
    }
    assert_pull_is_no_op(
        &b.service(),
        &storage_id,
        &a.detail.project.bundle_id,
        b.binding(),
    );
    let mut verify = b_threads
        .iter()
        .enumerate()
        .map(|(index, id)| (b.row(id), final_history(index + 1)))
        .collect::<Vec<_>>();
    let bindings = b
        .repository
        .load_conversation_bindings()
        .expect("load preserved isolation rows");
    for index in 3..5 {
        let preserved = bindings
            .bindings
            .iter()
            .filter(|row| {
                row.local_project_id == *b.id()
                    && row
                        .preserved
                        .as_ref()
                        .is_some_and(|p| p.preserved_from == clouds[index])
            })
            .collect::<Vec<_>>();
        assert_eq!(
            preserved.len(),
            1,
            "Pull retry duplicated its preserved branch"
        );
        assert!(preserved[0].cloud_thread_id.is_none());
        assert_eq!(
            preserved[0].provider_thread_id.as_ref(),
            Some(&b_aliases[index])
        );
        verify.push((preserved[0].clone(), preserved_history(index + 1)));
    }
    for (index, (row, expected)) in verify.iter().enumerate() {
        let alias = row
            .provider_thread_id
            .as_deref()
            .expect("verification alias");
        assert_resolved_local_alias_history(
            &b.codex_home,
            &b.project_root,
            alias,
            row.root_tail_layout.as_ref(),
            expected,
            VERIFY_REPLICA,
            index,
        );
        run_headless_history_verification(
            &mut turns,
            provider,
            &[],
            &b.codex_home,
            &b.project_root,
            alias,
            expected,
            VERIFY_REPLICA,
            index,
        );
    }
    assert_eq!(
        turns, EXPECTED_TURNS,
        "isolation changed its Codex turn budget"
    );
    let canonical = std::fs::canonicalize(root.path()).expect("canonicalize isolation root");
    assert_portable_state_is_safe(
        &a.service(),
        &storage_id,
        &a.detail.project.bundle_id,
        &storage_root,
        &[],
        &[
            root.path().to_string_lossy().as_bytes().to_vec(),
            canonical.to_string_lossy().as_bytes().to_vec(),
        ],
    );
    for replica in [&a, &b] {
        assert_codex_home_has_no_sensitive_state(&replica.codex_home, &[]);
        assert_process_home_has_no_sensitive_state(&replica.process_home, &[]);
    }
}
