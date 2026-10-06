// Mallard Cloud: a repository gone or being deleted, and a draft discarded
// (PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md), against the in-process stub of
// the service. A gone repository reads as a cleared storage does: stale
// records read as absent, nothing is reset, and the user pushes again.
mod managed_cloud_gone {
    use super::*;
    use mallard_domain::{BoundedConfigObject, SecretMap, SecretValue, StorageProviderConfig};
    use mallard_infrastructure::managed_cloud::client::{
        DraftState, ManagedCloudStatus, ManagedCloudStatusState,
    };
    use mallard_infrastructure::managed_cloud::stub::{StubManagedCloud, StubState};
    use mallard_infrastructure::storage_providers::managed_cloud::{
        provider_key, token_key, ManagedCloudSettingsV1,
    };
    use std::sync::{Arc, Mutex, MutexGuard};

    /// The stub, as a test body on a blocking thread reaches it.
    #[derive(Clone)]
    pub(super) struct Cloud {
        endpoint: String,
        state: Arc<Mutex<StubState>>,
    }

    impl Cloud {
        fn state(&self) -> MutexGuard<'_, StubState> {
            self.state.lock().unwrap()
        }

        /// A session of `login`, whose account the first one creates.
        fn sign_in(&self, login: &str) -> (String, String) {
            let mut state = self.state();
            let known = state
                .users
                .values()
                .find(|user| user.login == login)
                .map(|user| user.id.clone());
            match known {
                Some(id) => {
                    let token = state.add_session(&id);
                    (id, token)
                }
                None => {
                    let (user, token) = state.add_user(login);
                    (user.id, token)
                }
            }
        }

        /// The active repository of the project, as the service resolves it.
        fn repository_id(&self, bundle: &BundleId) -> String {
            self.state()
                .repository_for_bundle(bundle.as_str())
                .expect("the project has an active repository")
                .id
                .clone()
        }

        fn remove(&self, repository_id: &str, finished: bool) {
            let mut state = self.state();
            state.remove(repository_id);
            if finished {
                state.finish_removal(repository_id);
            }
        }

        fn clear_log(&self) {
            self.state().log.clear();
        }

        /// The requests since the log was cleared, as `METHOD path`.
        fn requests(&self) -> Vec<String> {
            self.state()
                .log
                .iter()
                .map(|request| format!("{} {}", request.method, request.path))
                .collect()
        }

        /// The repositories the requests since then wrote to.
        fn written_to(&self) -> BTreeSet<String> {
            self.requests()
                .iter()
                .filter(|line| !line.starts_with("GET "))
                .filter_map(|line| line.split('/').nth(3).map(str::to_owned))
                .collect()
        }
    }

    /// Run a test body against a fresh stub, on a thread that may block on
    /// the store's requests.
    pub(super) async fn with_cloud(body: impl FnOnce(Cloud) + Send + 'static) {
        let stub = StubManagedCloud::start().await;
        let cloud = Cloud {
            endpoint: stub.endpoint.clone(),
            state: stub.state.clone(),
        };
        tokio::task::spawn_blocking(move || body(cloud))
            .await
            .unwrap();
    }

    pub(super) fn add_cloud_storage(
        repo: &LocalStateRepository,
        cloud: &Cloud,
        login: &str,
        storage: &str,
    ) -> StorageId {
        let (account_id, token) = cloud.sign_in(login);
        let storage_id = StorageId::parse(storage).unwrap();
        let mut secrets = SecretMap::empty();
        secrets
            .insert(token_key(), SecretValue::parse(&token).unwrap())
            .unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
            id: storage_id.clone(),
            name: "Mallard Cloud".to_string(),
            provider: provider_key(),
            provider_config: StorageProviderConfig::new(
                1,
                BoundedConfigObject::encode(&ManagedCloudSettingsV1 {
                    service_origin: cloud.endpoint.clone(),
                    github_user_id: cloud.state().users[&account_id].github_user_id.clone(),
                    account_login: login.to_string(),
                })
                .unwrap(),
                secrets,
            ),
        });
        repo.save_config(config).unwrap();
        storage_id
    }

    /// One computer: its own state directory, its own session at the
    /// service and so its own store, and the project bound and linked.
    struct Computer {
        _temp: tempfile::TempDir,
        repo: LocalStateRepository,
        storage_id: StorageId,
        project: LocalProjectRegistration,
        root: PathBuf,
        codex: PathBuf,
        binding: ProjectBinding,
    }

    impl Computer {
        fn new(cloud: &Cloud, login: &str, storage: &str) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let repo = repository(&temp);
            let storage_id = add_cloud_storage(&repo, cloud, login, storage);
            let project = register(&repo);
            let root = temp.path().join("project");
            let codex = temp.path().join("codex-home");
            let binding = bind_and_link_test_replica(&repo, &project, &storage_id, &root, &codex);
            Self {
                _temp: temp,
                repo,
                storage_id,
                project,
                root,
                codex,
                binding,
            }
        }

        fn session(&self, id: &str) {
            write_codex_session(&self.codex, id, &self.root);
        }

        fn continue_session(&self, id: &str, text: &str) {
            append_codex_user_turn(
                &self
                    .codex
                    .join(format!("sessions/2026/07/22/rollout-{id}.jsonl")),
                &format!("message-{text}"),
                text,
            );
        }

        fn compare(&self) -> ThreadSyncComparison {
            get_project_thread_sync_comparison_with_repository(
                &self.repo,
                &self.project.local_project_id,
                &self.storage_id,
            )
            .unwrap()
        }

        fn cloud_status(&self) -> ManagedCloudStatus {
            self.compare()
                .managed_cloud_status
                .expect("a linked Cloud project reports its status")
        }

        fn push(&self) -> ProjectOperationResult {
            let result = self.try_push().unwrap();
            assert!(result.success, "{}", result.message);
            result
        }

        fn try_push(&self) -> Result<ProjectOperationResult, String> {
            push_bundle_with_repository(
                &self.repo,
                &self.project.local_project_id,
                &self.storage_id,
            )
        }

        fn push_status(&self) -> PushDestinationStatus {
            get_push_destination_status_with_repository(
                &self.repo,
                &self.project.local_project_id,
                &self.storage_id,
            )
            .unwrap()
        }

        fn plan_pull(&self) -> Result<RestorePlan, String> {
            ProjectSyncService::new(&self.repo).plan_bundle_restore(
                &self.storage_id,
                &self.project.bundle_id,
                &self.binding,
            )
        }

        /// Review the Pull and apply every session row that carries an
        /// action; with none, the review alone is what is saved.
        fn pull(&self) -> RestoreResult {
            let plan = self.plan_pull().unwrap();
            let result = ProjectSyncService::new(&self.repo)
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
                "{} — failed: {:?}",
                result.message, result.failed_actions
            );
            result
        }

        fn link(&self) -> ProjectStorageLink {
            let config = self.repo.load_config().unwrap();
            project_link(&config, &self.project.local_project_id, &self.storage_id)
                .unwrap()
                .clone()
        }

        fn pin(&self) -> String {
            self.link()
                .managed_cloud_repository_id
                .expect("the link names its repository")
        }

        fn config_bytes(&self) -> Vec<u8> {
            std::fs::read(self.repo.config_path()).unwrap()
        }

        /// Every file of the project and of the provider home, by content.
        fn files(&self) -> BTreeMap<PathBuf, Vec<u8>> {
            fn walk(directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
                let Ok(entries) = std::fs::read_dir(directory) else {
                    return;
                };
                for entry in entries {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        walk(&path, files);
                    } else {
                        files.insert(path.clone(), std::fs::read(&path).unwrap());
                    }
                }
            }
            let mut files = BTreeMap::new();
            walk(&self.root, &mut files);
            walk(&self.codex, &mut files);
            files
        }

        fn mainline(&self) -> Mainline {
            let (_, engine) = storage_engine(&self.repo, &self.storage_id).unwrap();
            engine.observe(&self.project.bundle_id).unwrap().mainline
        }

        /// The events of one activity refresh, as the workspace receives them.
        fn refresh_events(&self) -> Vec<ProjectActivityRefreshEvent> {
            let mut events = Vec::new();
            refresh_project_activity_with_progress(
                &self.repo,
                &self.project.local_project_id,
                &self.storage_id,
                None,
                None,
                false,
                &mut |event| {
                    events.push(event);
                    Ok(())
                },
                &mut |_, _| {},
            )
            .unwrap();
            events
        }
    }

    /// The state of every row, by the session it is on the computer that
    /// first pushed it: a computer that pulled a session holds it under a
    /// provider id of its own and the same cloud id.
    fn states(
        comparison: &ThreadSyncComparison,
        pusher: &Computer,
    ) -> BTreeMap<String, ThreadSyncState> {
        let (_, engine) = storage_engine(&pusher.repo, &pusher.storage_id).unwrap();
        let stored = engine
            .observe(&pusher.project.bundle_id)
            .unwrap()
            .snapshot()
            .map(stored_cloud_ids)
            .unwrap_or_default();
        comparison
            .entries
            .iter()
            .filter_map(|entry| {
                let session = stored
                    .iter()
                    .find(|(_, cloud_id)| entry.cloud_thread_id.as_ref() == Some(*cloud_id))
                    .and_then(|(session, _)| session.clone())
                    .or_else(|| entry.provider_thread_id.clone())?;
                Some((session, entry.state))
            })
            .collect()
    }

    const FIRST: &str = "019f7500-0000-7000-8000-00000000c001";
    const SECOND: &str = "019f7500-0000-7000-8000-00000000c002";
    const THIRD: &str = "019f7500-0000-7000-8000-00000000c003";

    /// The incident of 2026-09-26, turned around: a repository removed on the
    /// website, read while it is being deleted and after it is gone. The
    /// comparison succeeds, every session reads its own state, Push is open,
    /// and the refresh saves nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refresh_of_a_gone_repository_saves_nothing_and_reads_every_session_local_only() {
        with_cloud(|cloud| {
            for (login, finished) in [("rika", false), ("tom", true)] {
                let a = Computer::new(&cloud, login, &format!("cloud-gone-refresh-{login}"));
                a.session(FIRST);
                assert_eq!(a.push().revision, Some(2));
                // Both accounts hold a repository of this project; each
                // computer's own is the one its link names.
                let old = a.pin();
                let ready = a.compare();
                assert_eq!(
                    ready.managed_cloud_status.as_ref().unwrap().state,
                    ManagedCloudStatusState::Ready
                );
                assert_eq!(
                    comparison_entry(&ready, FIRST).state,
                    ThreadSyncState::Synced
                );

                cloud.remove(&old, finished);
                let before = a.config_bytes();
                let reviewed = a.link();
                let files = a.files();
                cloud.clear_log();
                let comparison = a.compare();
                let status = comparison.managed_cloud_status.clone().unwrap();
                assert_eq!(
                    status.state,
                    if finished {
                        ManagedCloudStatusState::Gone
                    } else {
                        ManagedCloudStatusState::Deleting
                    }
                );
                assert_eq!(status.repository_id, old, "the status names the link's pin");
                assert!(status.link.is_none());
                assert_eq!(
                    comparison.remote_continuity,
                    RemoteContinuity::RemoteMissing { base_revision: 2 }
                );
                let row = comparison_entry(&comparison, FIRST);
                assert_eq!(row.state, ThreadSyncState::LocalOnly);
                assert_eq!(row.push_decision.blocker, None);
                assert_eq!(
                    row.push_decision.action,
                    ConversationReviewAction::CreateCloud
                );
                assert_eq!(
                    row.pull_decision.action,
                    ConversationReviewAction::ReportLocalOnly
                );
                assert_eq!(comparison.counts.unknown, 0);
                assert!(comparison
                    .scopes
                    .iter()
                    .all(|scope| scope.continuity == ScopeContinuity::Fresh));
                assert_eq!(
                    a.push_status(),
                    PushDestinationStatus::Ready { revision: None }
                );
                assert_eq!(
                    a.plan_pull().unwrap_err(),
                    "There is nothing to pull from Mallard Cloud."
                );
                // The workspace's refresh ends in a comparison, never an error.
                let events = a.refresh_events();
                assert!(events.iter().all(|event| !matches!(
                    event,
                    ProjectActivityRefreshEvent::ComparisonError { .. }
                        | ProjectActivityRefreshEvent::ActivityError { .. }
                )));
                let Some(ProjectActivityRefreshEvent::Comparison { comparison }) = events.last()
                else {
                    panic!("the refresh ends in its comparison");
                };
                assert_eq!(comparison.managed_cloud_status, Some(status));
                // Observing wrote nothing: not here, and not at the service.
                assert_eq!(a.config_bytes(), before);
                assert_eq!(a.link(), reviewed);
                assert_eq!(a.files(), files);
                assert!(cloud.written_to().is_empty(), "{:?}", cloud.requests());
            }
        })
        .await;
    }

    /// One Push creates the repository again, at the address it had, and the
    /// link names it from then on. Nothing else of the link moves: no unlink,
    /// no reset, no relink.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_push_after_removal_publishes_a_first_version_and_saves_the_new_pin() {
        with_cloud(|cloud| {
            for (login, finished) in [("rika", false), ("tom", true)] {
                let a = Computer::new(&cloud, login, &format!("cloud-gone-push-{login}"));
                a.session(FIRST);
                assert_eq!(a.push().revision, Some(2));
                let old = a.pin();
                let address = {
                    let link = a.cloud_status().link.unwrap();
                    format!("{}/{}", link.owner_login, link.name)
                };
                let reviewed = a.link();
                cloud.remove(&old, finished);
                a.compare();

                cloud.clear_log();
                let republished = a.push();
                assert_eq!(republished.revision, Some(1));
                assert_eq!(republished.message, "Published version 1");
                let new = a.pin();
                assert_ne!(new, old);
                assert_eq!(cloud.state().repositories[&new].state, "active");
                assert_eq!(cloud.written_to(), BTreeSet::from([new.clone()]));
                assert_eq!(
                    cloud
                        .requests()
                        .iter()
                        .filter(|line| *line == "POST /v1/repositories")
                        .count(),
                    1
                );

                let link = a.link();
                assert_eq!(
                    link.managed_cloud_repository_id.as_deref(),
                    Some(new.as_str())
                );
                let mainline = a.mainline();
                assert_eq!(mainline.names().len(), 1);
                assert!(repository_bases(&link)
                    .all(|base| mainline.contains(base.revision, &base.commit_id)));
                assert!(link
                    .conversation_checkpoints
                    .values()
                    .flat_map(BTreeMap::values)
                    .filter_map(|checkpoint| checkpoint.accepted.as_ref())
                    .all(|accepted| mainline.contains_commit(&accepted.accepted_at_commit_id)));
                // What the link chose and kept is what it was.
                assert_eq!(link.recipe, reviewed.recipe);
                assert_eq!(
                    link.project_content_preferences,
                    reviewed.project_content_preferences
                );
                assert_eq!(link.pinned, reviewed.pinned);
                assert_eq!(link.created_at, reviewed.created_at);
                // No record of a session was dropped or started over: each is
                // the one it was, accepted again in the new history.
                let checkpoints = |link: &ProjectStorageLink| {
                    link.conversation_checkpoints
                        .iter()
                        .flat_map(|(scope, threads)| {
                            threads.iter().map(move |(thread, checkpoint)| {
                                (
                                    (scope.clone(), thread.clone()),
                                    (checkpoint.generation, checkpoint.physical.clone()),
                                )
                            })
                        })
                        .collect::<BTreeMap<_, _>>()
                };
                let (now, then) = (checkpoints(&link), checkpoints(&reviewed));
                assert_eq!(
                    now.keys().collect::<Vec<_>>(),
                    then.keys().collect::<Vec<_>>()
                );
                for (session, (generation, physical)) in &now {
                    assert!(*generation > then[session].0);
                    assert_eq!(physical, &then[session].1);
                }

                // The next refresh reads the new repository alone, in sync,
                // at the address the old one had.
                cloud.clear_log();
                let settled = a.compare();
                assert!(cloud.requests().iter().all(|line| !line.contains(&old)));
                let status = settled.managed_cloud_status.clone().unwrap();
                assert_eq!(status.state, ManagedCloudStatusState::Ready);
                assert_eq!(status.repository_id, new);
                let cloud_link = status.link.unwrap();
                assert_eq!(cloud_link.repository_id, new);
                assert_eq!(
                    format!("{}/{}", cloud_link.owner_login, cloud_link.name),
                    address
                );
                assert_eq!(cloud_link.visibility, "private");
                assert_eq!(
                    settled.remote_continuity,
                    RemoteContinuity::InSync { revision: 1 }
                );
                assert_eq!(
                    comparison_entry(&settled, FIRST).state,
                    ThreadSyncState::Synced
                );
            }
        })
        .await;
    }

    /// Two computers of one account. The repository is removed, the first
    /// computer pushes it again, and the second, whose link still names the
    /// old one, reads the new one: it belongs to the same account. Its
    /// refresh saves nothing; its Pull saves the new pin.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pull_from_the_repository_another_computer_created_again_saves_its_pin() {
        with_cloud(|cloud| {
            let a = Computer::new(&cloud, "rika", "cloud-adopt-a");
            a.session(FIRST);
            assert_eq!(a.push().revision, Some(2));
            let b = Computer::new(&cloud, "rika", "cloud-adopt-b");
            let old = cloud.repository_id(&a.project.bundle_id);
            assert_eq!((a.pin(), b.pin()), (old.clone(), old.clone()));
            b.pull();
            assert_eq!(states(&b.compare(), &a)[FIRST], ThreadSyncState::Synced);

            cloud.remove(&old, true);
            a.session(SECOND);
            a.compare();
            assert_eq!(a.push().revision, Some(1));
            let new = cloud.repository_id(&a.project.bundle_id);
            assert_eq!(a.pin(), new);

            // B still names the old repository. It reads the new one.
            let before = b.config_bytes();
            cloud.clear_log();
            let comparison = b.compare();
            let status = comparison.managed_cloud_status.clone().unwrap();
            assert_eq!(status.state, ManagedCloudStatusState::Ready);
            assert_eq!(status.repository_id, old, "the status names the link's pin");
            assert_eq!(status.link.as_ref().unwrap().repository_id, new);
            assert_eq!(status.link.as_ref().unwrap().role.as_deref(), Some("owner"));
            assert_eq!(
                comparison.remote_continuity,
                RemoteContinuity::Replaced {
                    base_revision: 2,
                    remote_revision: 1
                }
            );
            let rows = states(&comparison, &a);
            assert_eq!(rows[FIRST], ThreadSyncState::Synced);
            assert_eq!(rows[SECOND], ThreadSyncState::StorageOnly);
            assert_eq!(comparison.counts.unknown, 0);
            assert!(matches!(
                b.push_status(),
                PushDestinationStatus::RequiresPull {
                    reason: PushDestinationPullReason::RemoteWithoutBase,
                    ..
                }
            ));
            assert_eq!(b.config_bytes(), before, "a refresh saves nothing");
            assert_eq!(b.pin(), old);
            assert!(cloud.written_to().is_empty(), "{:?}", cloud.requests());

            // The Pull installs A's new session and saves the pin with its bases.
            b.pull();
            assert_eq!(b.pin(), new);
            let settled = b.compare();
            let status = settled.managed_cloud_status.clone().unwrap();
            assert_eq!(status.repository_id, new);
            assert_eq!(status.link.unwrap().repository_id, new);
            assert_eq!(
                settled.remote_continuity,
                RemoteContinuity::InSync { revision: 1 }
            );
            assert!(states(&settled, &a)
                .values()
                .all(|state| *state == ThreadSyncState::Synced));
            // From here on B is an ordinary replica: its Push lands on A's.
            cloud.clear_log();
            b.session(THIRD);
            assert_eq!(b.push().revision, Some(2));
            assert_eq!(cloud.written_to(), BTreeSet::from([new.clone()]));
            assert!(cloud.requests().iter().all(|line| !line.contains(&old)));
        })
        .await;
    }

    /// Two projects of one bundle on one storage share its repository, so
    /// the Push of one saves the pin of both.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_links_of_one_bundle_on_one_storage_receive_the_same_pin() {
        with_cloud(|cloud| {
            let temp = tempfile::tempdir().unwrap();
            let repo = repository(&temp);
            let storage_id = add_cloud_storage(&repo, &cloud, "rika", "cloud-gone-two-links");
            let bundle_id = BundleId::parse("5eaf00d5eaf00d5eaf00d5eaf00d5eaf").unwrap();
            let checkout = |name: &str| {
                let project = register_local_project_with_repository(
                    &repo,
                    RegisterLocalProjectRequest {
                        display_name: format!("Project on {name}"),
                        repository_fingerprint: Some("a".repeat(64)),
                        bundle_id: Some(bundle_id.clone()),
                    },
                )
                .unwrap();
                let root = temp.path().join(format!("project-{name}"));
                let codex = temp.path().join(format!("codex-{name}"));
                bind_and_link_test_replica(&repo, &project, &storage_id, &root, &codex);
                (project, root, codex)
            };
            let (first, first_root, first_codex) = checkout("a");
            let (second, _, _) = checkout("b");
            let pins = || {
                let config = repo.load_config().unwrap();
                [&first, &second].map(|project| {
                    project_link(&config, &project.local_project_id, &storage_id)
                        .unwrap()
                        .managed_cloud_repository_id
                        .clone()
                        .unwrap()
                })
            };
            write_codex_session(&first_codex, FIRST, &first_root);
            push_bundle_with_repository(&repo, &first.local_project_id, &storage_id).unwrap();
            let old = cloud.repository_id(&bundle_id);
            assert_eq!(pins(), [old.clone(), old.clone()]);

            cloud.remove(&old, true);
            let republished =
                push_bundle_with_repository(&repo, &first.local_project_id, &storage_id).unwrap();
            assert!(republished.success, "{}", republished.message);
            assert_eq!(republished.revision, Some(1));
            let new = cloud.repository_id(&bundle_id);
            assert_ne!(new, old);
            assert_eq!(pins(), [new.clone(), new.clone()]);
            // The other project's own records were not touched by that Push.
            let comparison = get_project_thread_sync_comparison_with_repository(
                &repo,
                &second.local_project_id,
                &storage_id,
            )
            .unwrap();
            let status = comparison.managed_cloud_status.unwrap();
            assert_eq!(status.state, ManagedCloudStatusState::Ready);
            assert_eq!(status.repository_id, new);
        })
        .await;
    }

    /// The owner's rule: after the draft is discarded the sessions are
    /// compared with the public version. Public version A is push 2; the
    /// draft A1, push 3, continued one session and added another. After the
    /// discard the computer that pushed A1 reads A: the session A holds
    /// unchanged is in sync, the one it continued is ahead, the one only the
    /// draft held is local only, and none is unknown. It pulls, which
    /// installs nothing, and its Push lands on the public version.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn after_a_discard_the_sessions_are_compared_with_the_public_version() {
        with_cloud(|cloud| {
            let a = Computer::new(&cloud, "rika", "cloud-discard-a");
            a.session(FIRST);
            a.session(SECOND);
            assert_eq!(a.push().revision, Some(2));
            let id = a.pin();
            cloud.state().release(&id, None);
            // Another computer of the account pulls the public version.
            let b = Computer::new(&cloud, "rika", "cloud-discard-b");
            b.pull();
            let public = a.mainline().selected().cloned().unwrap();

            a.continue_session(SECOND, "continued-in-the-draft");
            a.session(THIRD);
            assert_eq!(a.push().revision, Some(3));
            let drafted = a.cloud_status();
            assert_eq!(drafted.publication.unwrap().draft_ahead, Some(1));
            let reviewed = a.link();
            assert!(repository_bases(&reviewed).all(|base| base.revision == 3));

            assert_eq!(cloud.state().discard(&id), Ok(Some(1)));
            let before = a.config_bytes();
            let files = a.files();
            cloud.clear_log();
            let comparison = a.compare();
            let status = comparison.managed_cloud_status.clone().unwrap();
            assert_eq!(status.state, ManagedCloudStatusState::Ready);
            assert_eq!(status.repository_id, id);
            assert_eq!(status.link.as_ref().unwrap().repository_id, id);
            assert_eq!(status.link.as_ref().unwrap().visibility, "public");
            let publication = status.publication.clone().unwrap();
            assert_eq!(
                (
                    publication.latest_version,
                    publication.draft_ahead,
                    publication.public_version
                ),
                (Some(1), Some(0), Some(1))
            );
            assert_eq!(
                comparison.remote_continuity,
                RemoteContinuity::Replaced {
                    base_revision: 3,
                    remote_revision: 2
                }
            );
            assert_eq!(comparison.revision, Some(2));
            let rows = states(&comparison, &a);
            assert_eq!(rows[FIRST], ThreadSyncState::Synced);
            assert_eq!(rows[SECOND], ThreadSyncState::LocalAhead);
            assert_eq!(rows[THIRD], ThreadSyncState::LocalOnly);
            assert_eq!(comparison.counts.unknown, 0);
            assert!(comparison.entries.iter().all(|entry| {
                entry.push_decision.action != ConversationReviewAction::Blocked
                    && entry.pull_decision.action != ConversationReviewAction::Blocked
            }));
            assert_eq!(
                comparison_entry(&comparison, SECOND).push_decision.action,
                ConversationReviewAction::Publish
            );
            assert_eq!(
                comparison_entry(&comparison, THIRD).push_decision.action,
                ConversationReviewAction::CreateCloud
            );
            // Nothing was reset, here or at the service.
            assert_eq!(a.config_bytes(), before);
            assert_eq!(a.link(), reviewed);
            assert_eq!(a.files(), files);
            assert!(cloud.written_to().is_empty(), "{:?}", cloud.requests());
            assert_eq!(a.mainline().selected(), Some(&public));

            // The computer that last pulled A is where it was: in sync.
            let unmoved = b.compare();
            assert_eq!(
                unmoved.remote_continuity,
                RemoteContinuity::InSync { revision: 2 }
            );
            assert!(states(&unmoved, &a)
                .values()
                .all(|state| *state == ThreadSyncState::Synced));
            assert_eq!(
                b.push_status(),
                PushDestinationStatus::Ready { revision: Some(2) }
            );

            // A reviews the history it had not: one Pull, nothing installed.
            assert!(matches!(
                a.push_status(),
                PushDestinationStatus::RequiresPull {
                    reason: PushDestinationPullReason::RemoteWithoutBase,
                    remote_revision: 2,
                    ..
                }
            ));
            let pulled = a.pull();
            assert_eq!(a.files(), files, "{}", pulled.message);
            assert_eq!(
                a.push_status(),
                PushDestinationStatus::Ready { revision: Some(2) }
            );
            // Its Push is a new draft on top of the public version.
            let pushed = a.push();
            assert_eq!(pushed.revision, Some(3));
            let mainline = a.mainline();
            let names = mainline.names();
            assert_eq!(names.len(), 3);
            assert_eq!(names[1], public);
            assert_eq!(names[2].parent(), Some(public.token()));
            let link = a.link();
            assert_eq!(
                link.managed_cloud_repository_id.as_deref(),
                Some(id.as_str())
            );
            assert!(repository_bases(&link)
                .all(|base| { base.revision == 3 && base.commit_id == names[2].commit_id() }));
            let settled = a.compare();
            assert_eq!(
                settled.remote_continuity,
                RemoteContinuity::InSync { revision: 3 }
            );
            assert!(states(&settled, &a)
                .values()
                .all(|state| *state == ThreadSyncState::Synced));
            let publication = settled.managed_cloud_status.unwrap().publication.unwrap();
            assert_eq!(
                (publication.draft_ahead, publication.public_version),
                (Some(1), Some(1))
            );
            // B reads ahead, and its Pull installs the two sessions.
            let behind = b.compare();
            assert_eq!(
                behind.remote_continuity,
                RemoteContinuity::Advanced {
                    base_revision: 2,
                    remote_revision: 3
                }
            );
            let rows = states(&behind, &a);
            assert_eq!(rows[SECOND], ThreadSyncState::StorageAhead);
            assert_eq!(rows[THIRD], ThreadSyncState::StorageOnly);
        })
        .await;
    }

    /// The desktop's discard verb: one request that names the repository by
    /// the link's pin. It reads nothing and writes nothing on this computer,
    /// and no other storage has it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_discard_verb_names_the_pin_sends_one_request_and_changes_nothing_here() {
        with_cloud(|cloud| {
            let a = Computer::new(&cloud, "rika", "cloud-discard-verb");
            let elsewhere = StorageId::parse("cloud-discard-verb-folder").unwrap();
            add_local_storage(
                &a.repo,
                &elsewhere,
                &a.root.parent().unwrap().join("folder-store"),
            );
            save_project_link_with_repository(
                &a.repo,
                SaveProjectLinkRequest {
                    local_project_id: a.project.local_project_id.clone(),
                    storage_id: elsewhere.clone(),
                    pinned: true,
                },
            )
            .unwrap();
            a.session(FIRST);
            assert_eq!(a.push().revision, Some(2));
            let id = a.pin();
            cloud.state().release(&id, None);
            a.session(SECOND);
            assert_eq!(a.push().revision, Some(3));
            let revision = a.cloud_status().link.unwrap().repository_revision;
            let discard = |storage: &StorageId, revision: u64| {
                tokio::runtime::Handle::current().block_on(
                    crate::project_sync::managed_cloud_link::discard_draft(
                        &a.repo,
                        &a.project.local_project_id,
                        storage,
                        revision,
                    ),
                )
            };
            let before = a.config_bytes();
            let files = a.files();
            cloud.clear_log();

            assert_eq!(
                discard(&elsewhere, revision).unwrap_err(),
                "This storage is not Mallard Cloud."
            );
            assert!(cloud.requests().is_empty());
            // A stale revision is refused, and the draft is what it was.
            let refused = discard(&a.storage_id, revision + 1).unwrap_err();
            assert!(
                refused.contains("The repository changed since you read it"),
                "{refused}"
            );
            assert_eq!(a.mainline().names().len(), 3);

            cloud.clear_log();
            let answer = discard(&a.storage_id, revision).unwrap();
            assert_eq!(
                cloud.requests(),
                vec![format!(
                    "POST /v1/repositories/{id}/private-snapshot/discard"
                )]
            );
            assert_eq!(answer.repository_id, id);
            assert_eq!(answer.state, "done");
            assert_eq!(answer.kept_version, Some(1));
            assert_eq!(a.config_bytes(), before);
            assert_eq!(a.files(), files);

            // What follows is the refresh's: the draft is the public version.
            let comparison = a.compare();
            assert_eq!(
                comparison.remote_continuity,
                RemoteContinuity::Replaced {
                    base_revision: 3,
                    remote_revision: 2
                }
            );
            let rows = states(&comparison, &a);
            assert_eq!(rows[FIRST], ThreadSyncState::Synced);
            assert_eq!(rows[SECOND], ThreadSyncState::LocalOnly);
            assert_eq!(a.config_bytes(), before);
        })
        .await;
    }

    /// The desktop's Publish verb: one listing of the project's stream, then
    /// one request that names the draft the listing selected, the repository
    /// by the link's pin, and the revision the listing carried. It reads
    /// nothing and writes nothing on this computer, and no other storage has
    /// it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_publish_verb_lists_once_names_the_draft_and_changes_nothing_here() {
        with_cloud(|cloud| {
            let a = Computer::new(&cloud, "rika", "cloud-publish-verb");
            let elsewhere = StorageId::parse("cloud-publish-verb-folder").unwrap();
            add_local_storage(
                &a.repo,
                &elsewhere,
                &a.root.parent().unwrap().join("folder-store"),
            );
            save_project_link_with_repository(
                &a.repo,
                SaveProjectLinkRequest {
                    local_project_id: a.project.local_project_id.clone(),
                    storage_id: elsewhere.clone(),
                    pinned: true,
                },
            )
            .unwrap();
            a.session(FIRST);
            assert_eq!(a.push().revision, Some(2));
            let id = a.pin();
            let publish = |storage: &StorageId, name: Option<&str>| {
                tokio::runtime::Handle::current().block_on(
                    crate::project_sync::managed_cloud_link::publish_version(
                        &a.repo,
                        &a.project.local_project_id,
                        storage,
                        name,
                    ),
                )
            };
            let before = a.config_bytes();
            let files = a.files();
            cloud.clear_log();

            // Another storage has no versions, and a name too long is
            // refused before any request.
            assert_eq!(
                publish(&elsewhere, None).unwrap_err(),
                "This storage is not Mallard Cloud."
            );
            assert_eq!(
                publish(&a.storage_id, Some(&"n".repeat(81))).unwrap_err(),
                "A version's name holds at most 80 characters."
            );
            assert!(cloud.requests().is_empty());

            let draft = a.mainline().selected().cloned().unwrap();
            cloud.clear_log();
            let answer = publish(&a.storage_id, Some("  First cut ")).unwrap();
            assert_eq!(
                cloud.requests(),
                vec![
                    format!("GET /v1/repositories/{id}/store"),
                    format!("POST /v1/repositories/{id}/versions"),
                ]
            );
            assert_eq!(
                (answer.repository_id.as_str(), answer.version),
                (id.as_str(), 1)
            );
            {
                let state = cloud.state();
                let version = state.repositories[&id].latest().unwrap();
                assert_eq!(version.record_name, draft.file_name());
                assert_eq!(version.name.as_deref(), Some("First cut"));
            }
            assert_eq!(a.config_bytes(), before);
            assert_eq!(a.files(), files);

            // What follows is the refresh's: the draft is version 1, private
            // as it was, and Publish is off until a push.
            let status = a.compare().managed_cloud_status.unwrap();
            let link = status.link.unwrap();
            assert_eq!(link.visibility, "private");
            assert!(!link.can_publish_version);
            let publication = status.publication.unwrap();
            assert_eq!(
                (
                    publication.latest_version,
                    publication.next_version,
                    publication.draft_ahead
                ),
                (Some(1), 2, Some(0))
            );
            let refused = publish(&a.storage_id, None).unwrap_err();
            assert!(
                refused.contains("The draft is already version 1."),
                "{refused}"
            );
            assert_eq!(cloud.state().repositories[&id].versions.len(), 1);

            // A push moves the draft, and the next Publish is version 2.
            a.session(SECOND);
            assert_eq!(a.push().revision, Some(3));
            assert!(a.cloud_status().link.unwrap().can_publish_version);
            assert_eq!(publish(&a.storage_id, Some(" ")).unwrap().version, 2);
            assert_eq!(cloud.state().repositories[&id].latest().unwrap().name, None);
        })
        .await;
    }

    /// A private repository without a version keeps no push, so its discard
    /// reads as a cleared storage does: nothing to compare with, Push open,
    /// a first push.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn after_a_discard_on_a_private_repository_without_a_version_a_push_starts_over() {
        with_cloud(|cloud| {
            let a = Computer::new(&cloud, "rika", "cloud-discard-private");
            a.session(FIRST);
            assert_eq!(a.push().revision, Some(2));
            let id = a.pin();
            assert_eq!(cloud.state().discard(&id), Ok(None));
            let before = a.config_bytes();
            let comparison = a.compare();
            let status = comparison.managed_cloud_status.clone().unwrap();
            assert_eq!(status.state, ManagedCloudStatusState::Ready);
            assert_eq!(status.link.unwrap().repository_id, id);
            assert_eq!(status.publication.unwrap().draft, DraftState::None);
            assert_eq!(
                comparison.remote_continuity,
                RemoteContinuity::RemoteMissing { base_revision: 2 }
            );
            assert_eq!(
                comparison_entry(&comparison, FIRST).state,
                ThreadSyncState::LocalOnly
            );
            assert_eq!(
                a.push_status(),
                PushDestinationStatus::Ready { revision: None }
            );
            assert_eq!(a.config_bytes(), before);
            assert_eq!(a.push().revision, Some(1));
            assert_eq!(a.pin(), id, "the repository is the one it was");
        })
        .await;
    }
}
