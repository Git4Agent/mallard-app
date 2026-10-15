//! Compression transitions over completed native AIMock transcripts.
//! Each case owns disposable provider homes and a separate storage stream.

use super::*;
use mallard_infrastructure::app_settings::{save_app_settings, AppSettings};
use mallard_infrastructure::jsonl_tail_io::compress_like_codex;

struct Machine {
    repository: LocalStateRepository,
    home: PathBuf,
    detail: ProjectDetail,
}

impl Machine {
    fn service(&self) -> ProjectSyncService<'_> {
        ProjectSyncService::new(&self.repository)
    }

    fn id(&self) -> &LocalProjectId {
        &self.detail.project.local_project_id
    }

    fn local(&self, cloud: &CloudThreadId) -> ThreadId {
        local_thread_of_cloud(&self.repository, self.id(), cloud)
    }

    fn alias(&self, cloud: &CloudThreadId) -> String {
        provider_thread_id(&self.repository, self.id(), &self.local(cloud))
    }

    fn history(&self, cloud: &CloudThreadId, expected: &[HistoryMarker]) {
        let resource =
            mallard_domain::conversation_resource_id(&codex_key(), &self.local(cloud)).unwrap();
        reverted_rollouts::assert_detail_history(&self.service(), self.id(), &resource, expected);
    }

    fn compare(&self, storage: &StorageId) -> ThreadSyncComparison {
        self.service()
            .compare_threads(self.id(), storage)
            .expect("compare compressed fixture")
    }

    fn state(&self, storage: &StorageId, cloud: &CloudThreadId, expected: ThreadSyncState) {
        // One comparison, with no sleep, cache invalidation, or retry.
        let comparison = self.compare(storage);
        let row = &comparison.entries[row_index(&comparison, cloud)];
        assert_eq!(row.state, expected, "compressed fixture comparison state");
        assert!(row.local_present);
        assert_eq!(
            row.provider_thread_id.as_deref(),
            Some(self.alias(cloud).as_str())
        );
        if expected == ThreadSyncState::Synced {
            assert_eq!(row.push_decision.blocker, None);
            assert_eq!(row.pull_decision.blocker, None);
        }
    }

    fn accepted(&self, storage: &StorageId) -> Value {
        let config = self.repository.load_config().unwrap();
        let link = config
            .links
            .iter()
            .find(|link| &link.storage_id == storage && &link.local_project_id == self.id())
            .expect("compression fixture link");
        // Observation cursors may advance during a failed Push. Acceptance
        // must remain at the last successful publication.
        let accepted = link
            .conversation_checkpoints
            .iter()
            .map(|(scope, checkpoints)| {
                let checkpoints = checkpoints
                    .iter()
                    .map(|(thread, checkpoint)| {
                        (
                            thread,
                            json!({
                                "accepted": checkpoint.accepted,
                                "local_tail": checkpoint.accepted_local_tail,
                                "subagents": checkpoint.accepted_subagents,
                            }),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                (scope, checkpoints)
            })
            .collect::<BTreeMap<_, _>>();
        serde_json::to_value(accepted).unwrap()
    }
}

fn row_index(comparison: &ThreadSyncComparison, cloud: &CloudThreadId) -> usize {
    let rows = comparison
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.cloud_thread_id.as_ref() == Some(cloud))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert_eq!(
        rows.len(),
        1,
        "compression duplicated or lost a bound session"
    );
    rows[0]
}

struct Fixture {
    _root: tempfile::TempDir,
    storage: StorageConfig,
    a: Machine,
    b: Machine,
}

impl Fixture {
    fn new(
        parent: &Path,
        project: &Path,
        sources: &[(&GraphNode, &[u8])],
        compressed_ids: &[&str],
    ) -> Self {
        let root = tempfile::tempdir_in(parent).expect("compression fixture root");
        let a_home = root.path().join("codex-a");
        let b_home = root.path().join("codex-b");
        create_secure_codex_home(&a_home);
        create_secure_codex_home(&b_home);
        let sessions = a_home.join("sessions/2026/10/15");
        std::fs::create_dir_all(&sessions).unwrap();
        for (node, bytes) in sources {
            let plain = sessions.join(node.path.file_name().unwrap());
            std::fs::write(&plain, bytes).unwrap();
            if compressed_ids.contains(&node.provider_thread_id.as_str()) {
                compress(&plain);
            }
        }
        let b_project = root.path().join("project-b");
        std::fs::create_dir(&b_project).unwrap();
        let a_repo = LocalStateRepository::from_home_dir(&root.path().join("machine-a")).unwrap();
        let b_repo = LocalStateRepository::from_home_dir(&root.path().join("machine-b")).unwrap();
        for repository in [&a_repo, &b_repo] {
            save_app_settings(
                repository,
                &AppSettings {
                    credential_redaction_enabled: true,
                    ..AppSettings::default()
                },
            )
            .unwrap();
        }
        let storage = local_storage(&root.path().join("store"));
        let a_detail = finish_setup(
            &a_repo,
            &ProjectSyncService::new(&a_repo),
            project,
            &a_home,
            storage.clone(),
            DraftRepositoryChoice::New,
        );
        let b_detail = finish_setup(
            &b_repo,
            &ProjectSyncService::new(&b_repo),
            &b_project,
            &b_home,
            storage.clone(),
            DraftRepositoryChoice::Existing {
                storage_id: storage.id.clone(),
                bundle_id: a_detail.project.bundle_id.clone(),
                display_name: a_detail.project.display_name.clone(),
                repository_fingerprint: a_detail.project.repository_fingerprint.clone(),
                mismatch_acknowledged: true,
            },
        );
        Self {
            _root: root,
            storage,
            a: Machine {
                repository: a_repo,
                home: a_home,
                detail: a_detail,
            },
            b: Machine {
                repository: b_repo,
                home: b_home,
                detail: b_detail,
            },
        }
    }

    fn publish(&self) -> u64 {
        let result = push(&self.a.service(), self.a.id(), &self.storage.id);
        assert!(result.success, "compressed fixture Push failed");
        result.revision.unwrap()
    }

    fn pull(&self) {
        pull_and_apply(
            &self.b.service(),
            &self.storage.id,
            &self.a.detail.project.bundle_id,
            self.b.detail.binding.as_ref().unwrap(),
        );
    }

    fn cloud(&self, node: &GraphNode) -> CloudThreadId {
        let local = logical_thread_id(&self.a.repository, self.a.id(), &node.provider_thread_id);
        cloud_thread_id_of(&self.a.repository, self.a.id(), &local)
    }
}

/// Match Codex's completed compression: pledged size, retained mtime, and
/// removal of the plain file only after the compressed sibling is durable.
fn compress(plain: &Path) -> PathBuf {
    let bytes = read_bounded_file(plain, MAX_CONVERSATION_FILE_BYTES, "compression source");
    let modified = std::fs::metadata(plain).unwrap().modified().unwrap();
    let compressed = plain.with_file_name(format!(
        "{}.zst",
        plain.file_name().unwrap().to_str().unwrap()
    ));
    assert!(!compressed.exists(), "compression target already exists");
    let staged = compressed.with_extension("zst.tmp");
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staged)
        .unwrap();
    file.write_all(&compress_like_codex(&bytes)).unwrap();
    file.set_modified(modified).unwrap();
    file.sync_all().unwrap();
    drop(file);
    std::fs::rename(staged, &compressed).unwrap();
    std::fs::remove_file(plain).unwrap();
    assert_eq!(
        std::fs::metadata(&compressed).unwrap().modified().unwrap(),
        modified
    );
    assert!(!plain.exists());
    compressed
}

fn append_completed(plain: &Path, prefix: &[u8], complete: &[u8]) {
    assert!(complete.len() > prefix.len() && complete.starts_with(prefix));
    assert_eq!(std::fs::read(plain).unwrap(), prefix);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(plain)
        .unwrap();
    file.write_all(&complete[prefix.len()..]).unwrap();
    file.sync_all().unwrap();
}

fn passed(name: &str) {
    eprintln!("compressed rollout case passed: {name}");
}

pub(super) fn run(parent: &Path, project: &Path, graph: &PaginatedGraph) {
    let started = Instant::now();
    eprintln!("live Codex phase: compressed rollouts (7 cases)");
    let original = graph
        .nodes
        .iter()
        .map(|(name, node)| {
            (
                *name,
                read_bounded_file(
                    &node.path,
                    MAX_CONVERSATION_FILE_BYTES,
                    "native compression seed",
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let root = &graph.nodes["p0"];
    let full = &original["p0"];
    // P1 retains exactly P0's first completed turn. Its native byte cutoff
    // supplies a replay boundary without synthesizing provider records.
    let metadata: Value =
        serde_json::from_slice(original["p1"].split(|b| *b == b'\n').next().unwrap()).unwrap();
    let end = metadata["payload"]["history_base"]["end_byte_offset"]
        .as_u64()
        .unwrap() as usize;
    assert!(end > 0 && end < full.len() && full[end - 1] == b'\n');
    let prefix = &full[..end];
    let first_turn = &root.expected[..2];
    assert_eq!(
        timeline_history_from_standalone(prefix, project, &root.provider_thread_id).unwrap(),
        first_turn
    );

    // First discovery sees only a compressed source, before any plain-file
    // metadata or comparison has been cached in this isolated home.
    {
        let fixture = Fixture::new(
            parent,
            project,
            &[(root, full)],
            &[&root.provider_thread_id],
        );
        let compressed = rollout_path(&fixture.a.home, &root.provider_thread_id);
        let saved = std::fs::read(&compressed).unwrap();
        let comparison = fixture.a.compare(&fixture.storage.id);
        assert_eq!(comparison.entries.len(), 1);
        assert_eq!(comparison.entries[0].state, ThreadSyncState::LocalOnly);
        assert!(comparison.entries[0].local_present);
        assert_eq!(fixture.publish(), 2);
        let cloud = fixture.cloud(root);
        fixture.a.history(&cloud, &root.expected);
        fixture.pull();
        fixture.b.history(&cloud, &root.expected);
        fixture
            .a
            .state(&fixture.storage.id, &cloud, ThreadSyncState::Synced);
        fixture
            .b
            .state(&fixture.storage.id, &cloud, ThreadSyncState::Synced);
        let installed = rollout_path(&fixture.b.home, &fixture.b.alias(&cloud));
        assert_eq!(installed.extension().unwrap(), "jsonl");
        assert_eq!(std::fs::read(&compressed).unwrap(), saved);
        assert!(!compressed.with_extension("").exists());
    }
    passed("compressed source at first discovery and Push/Pull");

    let quiet_id = "019fb029-0000-7000-8000-000000000098";
    let quiet = GraphNode {
        provider_thread_id: quiet_id.into(),
        path: root
            .path
            .with_file_name(format!("rollout-2026-10-15T00-00-00-{quiet_id}.jsonl")),
        expected: first_turn.to_vec(),
    };
    let quiet_bytes = String::from_utf8(prefix.to_vec())
        .unwrap()
        .replace(&root.provider_thread_id, quiet_id)
        .into_bytes();
    {
        let fixture = Fixture::new(
            parent,
            project,
            &[(root, prefix), (&quiet, &quiet_bytes)],
            &[],
        );
        assert_eq!(fixture.publish(), 2);
        fixture.pull();
        let cloud = fixture.cloud(root);
        let quiet_cloud = fixture.cloud(&quiet);
        let local = fixture.a.local(&cloud);
        let plain = rollout_path(&fixture.a.home, &root.provider_thread_id);
        let quiet_path = rollout_path(&fixture.a.home, quiet_id);
        fixture
            .a
            .state(&fixture.storage.id, &cloud, ThreadSyncState::Synced);
        fixture.a.history(&cloud, first_turn);
        let compressed = compress(&plain);
        let compressed_bytes = std::fs::read(&compressed).unwrap();
        fixture
            .a
            .state(&fixture.storage.id, &cloud, ThreadSyncState::Synced);
        assert_eq!(fixture.a.local(&cloud), local);
        assert_eq!(fixture.a.alias(&cloud), root.provider_thread_id);
        fixture.a.history(&cloud, first_turn);
        let storage_before = storage_digest_inventory(&fixture._root.path().join("store"));
        assert_eq!(fixture.publish(), 2);
        assert_eq!(
            storage_digest_inventory(&fixture._root.path().join("store")),
            storage_before
        );
        assert_eq!(fixture.a.compare(&fixture.storage.id).entries.len(), 2);
        fixture
            .a
            .state(&fixture.storage.id, &quiet_cloud, ThreadSyncState::Synced);
        passed("compression after cached comparison remains Synced and Push is a no-op");

        // Codex can leave the old compressed sibling while materializing its
        // plain writer. An appended native turn makes the chosen source observable.
        std::fs::write(&plain, prefix).unwrap();
        append_completed(&plain, prefix, full);
        fixture
            .a
            .state(&fixture.storage.id, &cloud, ThreadSyncState::LocalAhead);
        fixture.a.history(&cloud, &root.expected);
        assert_eq!(fixture.a.compare(&fixture.storage.id).entries.len(), 2);
        assert_eq!(std::fs::read(&compressed).unwrap(), compressed_bytes);
        passed("plain sibling wins without duplicate sessions");

        std::fs::remove_file(&compressed).unwrap();
        let b_local = fixture.b.local(&cloud);
        let b_plain = rollout_path(&fixture.b.home, &fixture.b.alias(&cloud));
        let b_compressed = compress(&b_plain);
        let b_saved = std::fs::read(&b_compressed).unwrap();
        fixture
            .a
            .state(&fixture.storage.id, &cloud, ThreadSyncState::LocalAhead);
        assert_eq!(fixture.publish(), 3);
        fixture
            .b
            .state(&fixture.storage.id, &cloud, ThreadSyncState::StorageAhead);
        fixture.pull();
        let installed = rollout_path(&fixture.b.home, &fixture.b.alias(&cloud));
        assert_ne!(installed, b_plain);
        assert_eq!(installed.extension().unwrap(), "jsonl");
        assert!(!b_plain.exists());
        assert_eq!(std::fs::read(&b_compressed).unwrap(), b_saved);
        assert_eq!(fixture.b.local(&cloud), b_local);
        for machine in [&fixture.a, &fixture.b] {
            machine.state(&fixture.storage.id, &cloud, ThreadSyncState::Synced);
            machine.state(&fixture.storage.id, &quiet_cloud, ThreadSyncState::Synced);
            machine.history(&cloud, &root.expected);
            machine.history(&quiet_cloud, &quiet.expected);
        }
        assert_eq!(std::fs::read(&quiet_path).unwrap(), quiet_bytes);
        passed("plain resume and Pull onto a compressed local copy");
    }

    let sources = graph
        .nodes
        .iter()
        .map(|(name, node)| (node, original[name].as_slice()))
        .collect::<Vec<_>>();
    for names in [&["p0"][..], &["p3"][..], &["p0", "p1", "p3"][..]] {
        let compressed = names
            .iter()
            .map(|name| graph.nodes[name].provider_thread_id.as_str())
            .collect::<Vec<_>>();
        let fixture = Fixture::new(parent, project, &sources, &compressed);
        assert_eq!(fixture.publish(), 2);
        fixture.pull();
        for machine in [&fixture.a, &fixture.b] {
            assert_eq!(
                machine.compare(&fixture.storage.id).entries.len(),
                graph.nodes.len()
            );
            for node in graph.nodes.values() {
                let cloud = fixture.cloud(node);
                machine.state(&fixture.storage.id, &cloud, ThreadSyncState::Synced);
                machine.history(&cloud, &node.expected);
            }
        }
        eprintln!(
            "compressed rollout chain variant passed: {}",
            names.join("+")
        );
    }
    passed("mixed compressed bases and leaves retain exact paged history");

    {
        let fixture = Fixture::new(parent, project, &[(root, prefix)], &[]);
        assert_eq!(fixture.publish(), 2);
        fixture.pull();
        let cloud = fixture.cloud(root);
        let plain = rollout_path(&fixture.a.home, &root.provider_thread_id);
        append_completed(&plain, prefix, full);
        fixture
            .a
            .state(&fixture.storage.id, &cloud, ThreadSyncState::LocalAhead);
        let service = fixture.a.service();
        let review = service
            .prepare_push_review(fixture.a.id(), &fixture.storage.id)
            .unwrap();
        let approval = PushApproval::recommended(&review, &fixture.storage.id);
        let comparison = review.thread_comparison.as_ref().unwrap();
        assert!(
            comparison.entries[row_index(comparison, &cloud)]
                .push_decision
                .selectable
        );
        let accepted = fixture.a.accepted(&fixture.storage.id);
        // Opening the UI review does not pin a transcript: Apply captures
        // current content. Change the source after that capture and its
        // selected-conversation review, at the existing production barrier.
        let mut barriers = 0;
        let result = service.push_reviewed_with_progress(
            fixture.a.id(),
            &fixture.storage.id,
            approval,
            &mut |_| {},
            &mut |phase, _| {
                if phase == "review selected conversations" {
                    barriers += 1;
                    assert_eq!(barriers, 1, "compression barrier repeated");
                    compress(&plain);
                }
            },
        );
        assert_eq!(barriers, 1, "compression barrier was not reached");
        assert!(
            !matches!(result, Ok(ref result) if result.success),
            "Push accepted a source compressed after capture"
        );
        assert_eq!(fixture.a.accepted(&fixture.storage.id), accepted);
        assert_eq!(
            service
                .inspect_bundle(&fixture.storage.id, &fixture.a.detail.project.bundle_id)
                .unwrap()
                .manifest
                .revision,
            2
        );
        assert_eq!(fixture.publish(), 3);
        fixture.pull();
        fixture.b.history(&cloud, &root.expected);
        fixture
            .a
            .state(&fixture.storage.id, &cloud, ThreadSyncState::Synced);
        fixture
            .b
            .state(&fixture.storage.id, &cloud, ThreadSyncState::Synced);
    }
    passed("compression after capture rejects stale Push and fresh review succeeds");

    {
        let fixture = Fixture::new(
            parent,
            project,
            &[(root, full), (&quiet, &quiet_bytes)],
            &[],
        );
        assert_eq!(fixture.publish(), 2);
        fixture.pull();
        let cloud = fixture.cloud(root);
        let quiet_cloud = fixture.cloud(&quiet);
        let compressed = compress(&rollout_path(&fixture.a.home, &root.provider_thread_id));
        fixture
            .a
            .state(&fixture.storage.id, &cloud, ThreadSyncState::Synced);
        fixture.a.history(&cloud, &root.expected);
        let service = fixture.a.service();
        let good_review = service
            .prepare_push_review(fixture.a.id(), &fixture.storage.id)
            .unwrap();
        let good_comparison = good_review.thread_comparison.as_ref().unwrap();
        let resource = &good_comparison.entries[row_index(good_comparison, &cloud)].resource_id;
        let mut approval = PushApproval::recommended(&good_review, &fixture.storage.id);
        approval.recipe.entries.retain(|id, _| id == resource);
        assert!(approval.recipe.entries.contains_key(resource));
        let accepted = fixture.a.accepted(&fixture.storage.id);
        let good = std::fs::read(&compressed).unwrap();
        // Keep the valid frame header. Full decoding must detect the lost end.
        std::fs::write(&compressed, &good[..good.len() - 1]).unwrap();
        let comparison = fixture.a.compare(&fixture.storage.id);
        let damaged = &comparison.entries[row_index(&comparison, &cloud)];
        assert!(matches!(
            damaged.state,
            ThreadSyncState::Unknown | ThreadSyncState::Unavailable
        ));
        assert!(!damaged.push_decision.selectable);
        assert!(damaged.push_decision.blocker.is_some());
        assert_eq!(
            comparison.entries[row_index(&comparison, &quiet_cloud)].state,
            ThreadSyncState::Synced
        );
        let review = fixture
            .a
            .service()
            .prepare_push_review(fixture.a.id(), &fixture.storage.id)
            .unwrap();
        let comparison = review.thread_comparison.as_ref().unwrap();
        let blocked = &comparison.entries[row_index(comparison, &cloud)];
        assert!(!blocked.push_decision.selectable);
        let result = service.push_reviewed(fixture.a.id(), &fixture.storage.id, approval);
        assert!(
            !matches!(result, Ok(ref result) if result.success),
            "Push accepted a truncated stream"
        );
        assert_eq!(fixture.a.accepted(&fixture.storage.id), accepted);
        assert_eq!(
            service
                .inspect_bundle(&fixture.storage.id, &fixture.a.detail.project.bundle_id)
                .unwrap()
                .manifest
                .revision,
            2
        );
        fixture.a.history(&quiet_cloud, &quiet.expected);
        fixture.b.history(&cloud, &root.expected);
        fixture
            .b
            .state(&fixture.storage.id, &cloud, ThreadSyncState::Synced);
    }
    passed("truncated compressed source blocks Push without stale Synced state");

    for (name, node) in &graph.nodes {
        assert_eq!(
            std::fs::read(&node.path).unwrap(),
            original[name],
            "compression phase changed its native seed"
        );
    }
    eprintln!(
        "compressed rollouts: 7/7 cases passed in {:.2}s",
        started.elapsed().as_secs_f64()
    );
}

#[test]
fn generated_graph_covers_compressed_rollout_scenarios() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let home = root.path().join("codex");
    std::fs::create_dir(&project).unwrap();
    create_secure_codex_home(&home);
    let project = canonicalize(project).unwrap();
    let graph = write_paginated_graph(&home, &project);
    run(root.path(), &project, &graph);
}
