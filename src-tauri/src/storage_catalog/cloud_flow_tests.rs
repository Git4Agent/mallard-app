// Included by service::tests::managed_cloud_tests; every home and credential is a fixture.
fn cloud_service(fixture: &LibraryFixture) -> StorageCatalogService {
    StorageCatalogService::with_registry(
        &fixture.user_home,
        &fixture.user_home,
        Box::new(PanicRunner),
        fixture.registry.clone(),
    )
    .unwrap()
}

/// The one item a row lists, and the repository the page knows for it.
async fn only_item(
    fixture: &Arc<LibraryFixture>,
    scope: &CatalogScopeId,
) -> (CatalogResourceId, Option<String>) {
    let scope = scope.clone();
    blocking(fixture, move |fixture| {
        let listed = page(fixture, &scope, Some(&fixture.storage.id)).unwrap();
        assert_eq!(listed.rows.len(), 1);
        let resource = listed.rows[0].resource_id.clone().unwrap();
        let repository = listed.cloud_repositories.get(&resource).cloned();
        (resource, repository)
    })
    .await
}

fn found(resolution: super::cloud::CatalogCloudResolution) -> super::cloud::CatalogCloudRepository {
    match resolution {
        super::cloud::CatalogCloudResolution::Found { repository } => *repository,
        other => panic!("expected a repository, got {other:?}"),
    }
}

fn saved_link_revision(fixture: &LibraryFixture, scope: &CatalogScopeId) -> u64 {
    fixture
        .catalog
        .load_scope_links()
        .unwrap()
        .link_for(scope, &fixture.storage.id)
        .unwrap()
        .revision
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cloud_batch_failure_reports_completed_and_unattempted_items_and_pins_only_successes() {
    use mallard_infrastructure::managed_cloud::stub::Fault;
    let stub = StubManagedCloud::start().await;
    let root = tempfile::tempdir().unwrap();
    let machine = managed_machine(
        root.path(),
        &stub,
        "batch",
        &[
            ("a", b"---\nname: a\n---\nAlpha."),
            ("b", b"---\nname: b\n---\nBeta."),
            ("c", b"---\nname: c\n---\nCharlie."),
        ],
    );
    let remote = stub.state.clone();
    let (scope, result) = blocking(&machine, move |fixture| {
        let scope = skill_scope(fixture).scope.scope_id;
        link_scope(fixture, &scope, &fixture.storage.id, 3);
        let service = cloud_service(fixture).with_progress(move |event| {
            if event.completed == 1 {
                remote.lock().unwrap().fail_next_record = Some(Fault::Before);
            }
        });
        let plan = service.prepare_push(&scope, &fixture.storage.id).unwrap();
        let result = service
            .apply_push(CatalogPushApproval {
                plan_id: plan.plan_id,
                selected_resource_ids: plan
                    .rows
                    .iter()
                    .filter(|row| row.row.selectable)
                    .map(|row| row.row.resource_id.clone())
                    .collect(),
                acknowledged_warning_digests: plan.warning_digests,
            })
            .unwrap();
        (scope, result)
    })
    .await;
    assert!(result.published);
    assert!(result.stopped.is_some());
    assert_eq!(
        result
            .results
            .iter()
            .map(|row| row.state.as_str())
            .collect::<Vec<_>>(),
        ["published", "failed", "not_attempted"]
    );
    assert!(
        result.revision.is_none(),
        "a batch has no shared repository version"
    );
    let saved = machine.catalog.load_scope_links().unwrap();
    let link = saved.link_for(&scope, &machine.storage.id).unwrap();
    assert_eq!(link.accepted_items.len(), 1);
    assert_eq!(link.cloud_repositories.len(), 1);
    assert_eq!(saved.schema, 1);
    assert!(link
        .accepted_items
        .contains_key(&result.results[0].resource_id));
    assert!(!link
        .accepted_items
        .contains_key(&result.results[1].resource_id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cloud_pet_connection_preview_pull_release_and_discard_share_the_repository() {
    use mallard_infrastructure::managed_cloud::client::ReleaseState;
    let stub = StubManagedCloud::start().await;
    let root = tempfile::tempdir().unwrap();
    let alice = managed_machine(root.path(), &stub, "pet-owner", &[]);
    let bob = managed_machine(root.path(), &stub, "pet-reader", &[]);
    let alice_scope = blocking(&alice, |fixture| {
        let pet = fixture.user_home.join("codex-home/pets/duck");
        fs::create_dir_all(&pet).unwrap();
        fs::write(
            pet.join("pet.json"),
            super::super::super::codex_pet::fixtures::manifest(
                2,
                "duck",
                "Duck",
                "Fixture duck",
                "spritesheet.png",
            ),
        )
        .unwrap();
        fs::write(
            pet.join("spritesheet.png"),
            super::super::super::codex_pet::fixtures::decodable_png_atlas(2),
        )
        .unwrap();
        let scope = create_scope(
            &fixture.registry,
            &fixture.catalog,
            codex_pets(2),
            &fixture.profile.profile_id,
            2,
        )
        .unwrap()
        .scope
        .scope_id;
        link_scope(fixture, &scope, &fixture.storage.id, 3);
        push_all(fixture, &scope, &fixture.storage.id);
        scope
    })
    .await;
    let alice_service = cloud_service(&alice);
    let (resource, learned) = only_item(&alice, &alice_scope).await;
    let resource = &resource;
    let listed = alice_service
        .list_cloud_repositories(&alice_scope, &alice.storage.id, resource)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    let repository = &listed[0];
    let id = &repository.repository_id;
    assert_eq!(learned.as_ref(), Some(id));
    assert_eq!(repository.address, "pet-owner/Duck");
    assert_eq!(repository.role.as_deref(), Some("owner"));
    assert_eq!(repository.display_name, "Duck");
    let resolved = found(
        alice_service
            .resolve_cloud_repository(&alice.storage.id, &repository.address)
            .await
            .unwrap(),
    );
    assert_eq!(resolved.resource_id, *resource);
    assert_eq!(resolved.coordinate, codex_pets(2));
    let initial = alice_service
        .cloud_status(&alice_scope, &alice.storage.id, resource, id)
        .await
        .unwrap();
    assert_eq!(
        initial.link.as_ref().unwrap().release,
        ReleaseState::Private
    );
    let revision = initial.link.unwrap().repository_revision;
    assert!(alice_service
        .request_cloud_release(&alice_scope, &alice.storage.id, resource, id, revision + 1)
        .await
        .is_err());
    alice_service
        .request_cloud_release(&alice_scope, &alice.storage.id, resource, id, revision)
        .await
        .unwrap();
    let invitation = {
        let mut state = stub.state.lock().unwrap();
        let owner = state.repositories[id].owner_id.clone();
        state.set_invite_code(id, &owner)
    };
    let bob_scope = blocking(&bob, |fixture| {
        create_scope(
            &fixture.registry,
            &fixture.catalog,
            codex_pets(2),
            &fixture.profile.profile_id,
            2,
        )
        .unwrap()
        .scope
        .scope_id
    })
    .await;
    let bob_service = cloud_service(&bob);
    let joined = found(
        bob_service
            .join_cloud_repository(&bob.storage.id, &invitation)
            .await
            .unwrap(),
    );
    assert_eq!(joined.resource_id, *resource);
    assert_eq!(joined.role.as_deref(), Some("member"));
    let link_revision = blocking(&bob, {
        let scope = bob_scope.clone();
        move |fixture| link_scope(fixture, &scope, &fixture.storage.id, 3).revision
    })
    .await;
    assert!(bob_service
        .connect_cloud_repository(&bob_scope, &bob.storage.id, resource, id, link_revision + 1)
        .await
        .is_err());
    bob_service
        .connect_cloud_repository(&bob_scope, &bob.storage.id, resource, id, link_revision)
        .await
        .unwrap();
    // Connecting writes only the binding. Preview fetches the item's stream;
    // neither the connection nor the preview installs anything.
    assert!(!bob.user_home.join("codex-home/pets/duck").exists());
    blocking(&bob, {
        let scope = bob_scope.clone();
        let resource = resource.clone();
        move |fixture| {
            let service = cloud_service(fixture);
            let preview = service
                .resource_preview(&scope, &fixture.storage.id, &resource)
                .unwrap();
            assert!(preview.starts_with("data:image/png;base64,"));
            assert!(!fixture.user_home.join("codex-home/pets/duck").exists());
            let plan = prepare_pull(fixture, &scope);
            let source = &plan.sources[&resource];
            assert_eq!(source.address, "pet-owner/Duck");
            assert_eq!(source.reads, "draft", "a member reads the draft");
            assert_eq!(source.revision, plan.item_heads[&resource].revision);
            let result = apply_pull_defaults(fixture, &plan).unwrap();
            assert!(result.remote_accepted);
            assert_eq!(result.results[0].outcome, CatalogActionOutcome::Installed);
        }
    })
    .await;
    assert!(bob.user_home.join("codex-home/pets/duck/pet.json").exists());
    stub.state.lock().unwrap().release(id, None);
    blocking(&alice, {
        let scope = alice_scope.clone();
        move |fixture| {
            fs::write(
                fixture.user_home.join("codex-home/pets/duck/pet.json"),
                super::super::super::codex_pet::fixtures::manifest(
                    2,
                    "duck",
                    "Duck",
                    "Changed duck",
                    "spritesheet.png",
                ),
            )
            .unwrap();
            push_all(fixture, &scope, &fixture.storage.id);
        }
    })
    .await;
    let status = alice_service
        .cloud_status(&alice_scope, &alice.storage.id, resource, id)
        .await
        .unwrap();
    let revision = status.link.unwrap().repository_revision;
    assert!(bob_service
        .discard_cloud_draft(&bob_scope, &bob.storage.id, resource, id, revision)
        .await
        .is_err());
    alice_service
        .discard_cloud_draft(&alice_scope, &alice.storage.id, resource, id, revision)
        .await
        .unwrap();
    let status = alice_service
        .cloud_status(&alice_scope, &alice.storage.id, resource, id)
        .await
        .unwrap();
    assert_eq!(status.link.unwrap().release, ReleaseState::Public);
    // Connecting the saved repository again changes nothing.
    let revision = saved_link_revision(&bob, &bob_scope);
    bob_service
        .connect_cloud_repository(&bob_scope, &bob.storage.id, resource, id, revision + 7)
        .await
        .unwrap();
    assert_eq!(saved_link_revision(&bob, &bob_scope), revision);
    assert_eq!(alice.catalog.load_scope_links().unwrap().schema, 1);
    assert_eq!(bob.catalog.load_scope_links().unwrap().schema, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cloud_public_repository_can_be_connected_without_account_membership() {
    let stub = StubManagedCloud::start().await;
    let root = tempfile::tempdir().unwrap();
    let owner = managed_machine(
        root.path(),
        &stub,
        "public-owner",
        &[("shared", b"---\nname: shared\n---\nPublic fixture.")],
    );
    let reader = managed_machine(root.path(), &stub, "public-reader", &[]);
    let owner_scope = blocking(&owner, |fixture| {
        let scope = skill_scope(fixture).scope.scope_id;
        link_scope(fixture, &scope, &fixture.storage.id, 3);
        push_all(fixture, &scope, &fixture.storage.id);
        scope
    })
    .await;
    let owner_service = cloud_service(&owner);
    let (resource, _) = only_item(&owner, &owner_scope).await;
    let rows = owner_service
        .list_cloud_repositories(&owner_scope, &owner.storage.id, &resource)
        .await
        .unwrap();
    let repository = &rows[0];
    stub.state
        .lock()
        .unwrap()
        .release(&repository.repository_id, None);
    let (reader_scope, revision) = blocking(&reader, |fixture| {
        let scope = skill_scope(fixture).scope.scope_id;
        let revision = link_scope(fixture, &scope, &fixture.storage.id, 3).revision;
        (scope, revision)
    })
    .await;
    let service = cloud_service(&reader);
    assert!(service
        .list_cloud_repositories(&reader_scope, &reader.storage.id, &resource)
        .await
        .unwrap()
        .is_empty());
    assert!(
        service
            .list_cloud_shared(&reader.storage.id)
            .await
            .unwrap()
            .is_empty(),
        "a public repository is no one's share"
    );
    // Starred and Public list it for the reader, by this row's coordinate
    // and, at the door, by any coordinate this build opens.
    let reader_id = {
        let mut state = stub.state.lock().unwrap();
        let reader_id = state
            .users
            .values()
            .find(|user| user.login == "public-reader")
            .unwrap()
            .id
            .clone();
        state.star(&reader_id, &repository.repository_id);
        state.describe(&repository.repository_id, "Public fixture skill.");
        reader_id
    };
    {
        use mallard_infrastructure::managed_cloud::client::PublicSort;
        let starred = service
            .list_cloud_starred(Some(&reader_scope), &reader.storage.id, None)
            .await
            .unwrap();
        assert_eq!(starred.rows.len(), 1);
        assert_eq!(starred.rows[0].address, repository.address);
        assert_eq!(starred.rows[0].role, None);
        assert!(starred.rows[0].viewer_starred);
        assert_eq!(
            starred.rows[0].description.as_deref(),
            Some("Public fixture skill.")
        );
        let listed = service
            .list_cloud_public(
                Some(&reader_scope),
                &reader.storage.id,
                PublicSort::Newest,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            listed
                .rows
                .iter()
                .map(|row| row.repository_id.as_str())
                .collect::<Vec<_>>(),
            vec![repository.repository_id.as_str()]
        );
        assert_eq!(listed.rows[0].star_count, 1);
        let any = service
            .list_cloud_public(None, &reader.storage.id, PublicSort::Stars, None)
            .await
            .unwrap();
        assert_eq!(
            any.rows.len(),
            1,
            "the door lists every kind this build opens"
        );
    }
    let public = found(
        service
            .resolve_cloud_repository(&reader.storage.id, &repository.address)
            .await
            .unwrap(),
    );
    assert_eq!(public.resource_id, repository.resource_id);
    assert_eq!(public.role, None);
    assert_eq!(public.visibility, "public");
    service
        .connect_cloud_repository(
            &reader_scope,
            &reader.storage.id,
            &public.resource_id,
            &public.repository_id,
            revision,
        )
        .await
        .unwrap();
    // Add put the public item on the reader's Catalog on the service, so
    // another computer's Library refresh lists it too.
    assert!(stub
        .state
        .lock()
        .unwrap()
        .catalog_added(&reader_id, &repository.repository_id));
    blocking(&reader, move |fixture| {
        let plan = prepare_pull(fixture, &reader_scope);
        let source = plan.sources.values().next().unwrap();
        assert_eq!(
            source.reads, "public",
            "anyone else reads the public version"
        );
        let outcome = apply_pull_defaults(fixture, &plan).unwrap();
        assert!(outcome.remote_accepted);
        assert_eq!(outcome.results[0].outcome, CatalogActionOutcome::Installed);
        assert!(fixture
            .user_home
            .join("codex-home/skills/shared/SKILL.md")
            .exists());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_cloud_connections_disambiguate_identical_items_and_isolate_profiles() {
    let stub = StubManagedCloud::start().await;
    let root = tempfile::tempdir().unwrap();
    let first = managed_machine(
        root.path(),
        &stub,
        "first-owner",
        &[("review", b"---\nname: review\n---\nFirst copy.")],
    );
    let second = managed_machine(
        root.path(),
        &stub,
        "second-owner",
        &[("review", b"---\nname: review\n---\nSecond copy.")],
    );
    let reader = managed_machine(root.path(), &stub, "two-profiles", &[]);
    let mut repositories = Vec::new();
    for owner in [&first, &second] {
        let scope = blocking(owner, |fixture| {
            let scope = skill_scope(fixture).scope.scope_id;
            link_scope(fixture, &scope, &fixture.storage.id, 3);
            push_all(fixture, &scope, &fixture.storage.id);
            scope
        })
        .await;
        let (resource, _) = only_item(owner, &scope).await;
        repositories.push(
            cloud_service(owner)
                .list_cloud_repositories(&scope, &owner.storage.id, &resource)
                .await
                .unwrap()
                .remove(0),
        );
    }
    assert_eq!(repositories[0].resource_id, repositories[1].resource_id);
    let (scope_a, scope_b) = blocking(&reader, |fixture| {
        let service = cloud_service(fixture);
        let first = skill_scope(fixture).scope.scope_id;
        let other_home = fixture.user_home.join("other-codex");
        fs::create_dir_all(other_home.join("skills")).unwrap();
        let profile = service
            .save_profile(
                ProviderKey::parse("codex").unwrap(),
                other_home.to_string_lossy().into(),
            )
            .unwrap();
        let second = create_scope(
            &fixture.registry,
            &fixture.catalog,
            codex_skills(),
            &profile.profile_id,
            2,
        )
        .unwrap()
        .scope
        .scope_id;
        (first, second)
    })
    .await;
    let service = cloud_service(&reader);
    for repository in &repositories {
        let invitation = {
            let mut state = stub.state.lock().unwrap();
            let owner = state.repositories[&repository.repository_id]
                .owner_id
                .clone();
            state.set_invite_code(&repository.repository_id, &owner)
        };
        service
            .join_cloud_repository(&reader.storage.id, &invitation)
            .await
            .unwrap();
    }
    let shared = service.list_cloud_shared(&reader.storage.id).await.unwrap();
    assert_eq!(
        shared.len(),
        2,
        "both shares are offered until a row saves one"
    );
    let (revision_a, revision_b) = blocking(&reader, {
        let a = scope_a.clone();
        let b = scope_b.clone();
        move |fixture| {
            // Two repositories hold the item: the page loads, and that row
            // waits for a choice instead of failing the page.
            let listed = page(fixture, &a, Some(&fixture.storage.id)).unwrap();
            assert_eq!(listed.rows.len(), 1);
            assert_eq!(listed.rows[0].state, Some(CatalogLinkState::Blocked));
            assert_eq!(
                listed.rows[0].blocker.as_deref(),
                Some("In 2 repositories you can see. Choose one on its page.")
            );
            assert!(listed.rows[0].remote_present);
            // Linking remains possible without inventory or remote acceptance.
            let revisions = (
                link_scope(fixture, &a, &fixture.storage.id, 3).revision,
                link_scope(fixture, &b, &fixture.storage.id, 3).revision,
            );
            // A Pull has nothing to move until it is chosen.
            assert_eq!(
                cloud_service(fixture)
                    .prepare_pull(&a, &fixture.storage.id, BTreeMap::new())
                    .unwrap_err(),
                "the storage has nothing of this coordinate to Pull"
            );
            revisions
        }
    })
    .await;
    for (scope, revision, repository) in [
        (&scope_a, revision_a, &repositories[0]),
        (&scope_b, revision_b, &repositories[1]),
    ] {
        service
            .connect_cloud_repository(
                scope,
                &reader.storage.id,
                &repository.resource_id,
                &repository.repository_id,
                revision,
            )
            .await
            .unwrap();
    }
    assert!(
        service
            .list_cloud_shared(&reader.storage.id)
            .await
            .unwrap()
            .is_empty(),
        "a share a row has saved is no longer offered"
    );
    blocking(&reader, move |fixture| {
        let read = |scope: &CatalogScopeId| {
            let plan = prepare_pull(fixture, scope);
            plan.items[0]
                .resource
                .as_ref()
                .unwrap()
                .content_sha256
                .clone()
        };
        let first_digest = read(&scope_a);
        let second_digest = read(&scope_b);
        assert_ne!(first_digest, second_digest);
        assert_eq!(
            read(&scope_a),
            first_digest,
            "a second profile must not change the first profile's routing or cache"
        );
        let links = fixture.catalog.load_scope_links().unwrap();
        assert_eq!(links.schema, 1);
        assert!(
            links
                .links
                .iter()
                .all(|link| link.accepted_items.is_empty()),
            "connection alone accepts no content"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_late_cancelled_review_cannot_discard_the_replacement_review() {
    use http_body_util::Full;
    use hyper::{body::Bytes, server::conn::http1, service::service_fn};
    use hyper_util::rt::TokioIo;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let reached = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let server = {
        let reached = reached.clone();
        let release = release.clone();
        let calls = calls.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let reached = reached.clone();
                let release = release.clone();
                let calls = calls.clone();
                tokio::spawn(async move {
                    let service =
                        service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                            let reached = reached.clone();
                            let release = release.clone();
                            let calls = calls.clone();
                            async move {
                                // The review's one display listing, naming
                                // where each item goes.
                                if request.uri().path() == "/v1/repositories" {
                                    return Ok::<_, std::convert::Infallible>(
                                        hyper::Response::new(Full::new(Bytes::from_static(
                                            b"{\"repositories\":[]}",
                                        ))),
                                    );
                                }
                                assert_eq!(request.uri().path(), "/v1/repositories/library");
                                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                                    reached.notify_one();
                                    release.notified().await;
                                }
                                Ok::<_, std::convert::Infallible>(hyper::Response::new(Full::new(
                                    Bytes::from_static(b"{\"repositories\":[]}"),
                                )))
                            }
                        });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                });
            }
        })
    };
    let mut stub = StubManagedCloud::start().await;
    stub.endpoint = endpoint;
    let root = tempfile::tempdir().unwrap();
    let machine = managed_machine(
        root.path(),
        &stub,
        "late-review",
        &[("review", b"---\nname: review\n---\nFixture.")],
    );
    let scope = blocking(&machine, |fixture| {
        let scope = skill_scope(fixture).scope.scope_id;
        link_scope(fixture, &scope, &fixture.storage.id, 3);
        scope
    })
    .await;
    let older = {
        let machine = machine.clone();
        let scope = scope.clone();
        tokio::spawn(async move {
            blocking(&machine, move |fixture| {
                prepare_push(fixture, &scope, &fixture.storage.id)
            })
            .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), reached.notified())
        .await
        .unwrap();
    let newer = blocking(&machine, move |fixture| {
        prepare_push(fixture, &scope, &fixture.storage.id)
    })
    .await;
    release.notify_one();
    let older = tokio::time::timeout(std::time::Duration::from_secs(5), older)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(older.plan_id, newer.plan_id);
    assert!(
        plan_store::load(machine.catalog.root(), &newer.plan_id).is_ok(),
        "the visible replacement must remain applicable when the cancelled call finishes"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "neither request retries automatically"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cloud_create_makes_one_private_repository_that_the_first_push_uses_and_keeps() {
    let stub = StubManagedCloud::start().await;
    let root = tempfile::tempdir().unwrap();
    let machine = managed_machine(
        root.path(),
        &stub,
        "creator",
        &[("review", b"---\nname: review\n---\nMine.")],
    );
    let other = managed_machine(
        root.path(),
        &stub,
        "other-owner",
        &[("review", b"---\nname: review\n---\nTheirs.")],
    );
    let scope = blocking(&machine, |fixture| {
        let scope = skill_scope(fixture).scope.scope_id;
        link_scope(fixture, &scope, &fixture.storage.id, 3);
        scope
    })
    .await;
    let (resource, learned) = only_item(&machine, &scope).await;
    assert_eq!(learned, None, "nothing holds the item before Create");
    let service = cloud_service(&machine);
    let revision = saved_link_revision(&machine, &scope);
    let created = service
        .create_cloud_repository(&scope, &machine.storage.id, &resource, revision)
        .await
        .unwrap();
    assert_eq!(created.address, "creator/review");
    assert_eq!(created.visibility, "private");
    assert_eq!(created.role.as_deref(), Some("owner"));
    assert!(created.registration_only);
    assert_eq!(created.resource_id, resource);
    let saved = machine.catalog.load_scope_links().unwrap();
    let link = saved.link_for(&scope, &machine.storage.id).unwrap();
    assert_eq!(
        link.cloud_repositories.get(&resource),
        Some(&created.repository_id)
    );
    assert!(
        link.accepted_items.is_empty(),
        "Create pulls and accepts nothing"
    );
    assert_eq!(saved.schema, 1);

    // Create again: the saved repository comes back, and nothing new exists.
    let again = service
        .create_cloud_repository(&scope, &machine.storage.id, &resource, revision)
        .await
        .unwrap();
    assert_eq!(again.repository_id, created.repository_id);
    let owned = |stub: &StubManagedCloud, login: &str| {
        let state = stub.state.lock().unwrap();
        state
            .repositories
            .values()
            .filter(|repository| state.users[&repository.owner_id].login == login)
            .count()
    };
    assert_eq!(owned(&stub, "creator"), 1);

    // The item keeps it: another repository of the same item is refused.
    let theirs_scope = blocking(&other, |fixture| {
        let scope = skill_scope(fixture).scope.scope_id;
        link_scope(fixture, &scope, &fixture.storage.id, 3);
        push_all(fixture, &scope, &fixture.storage.id);
        scope
    })
    .await;
    let (theirs_resource, theirs) = only_item(&other, &theirs_scope).await;
    assert_eq!(theirs_resource, resource);
    let theirs = theirs.unwrap();
    let invitation = {
        let mut state = stub.state.lock().unwrap();
        let owner = state.repositories[&theirs].owner_id.clone();
        state.set_invite_code(&theirs, &owner)
    };
    service
        .join_cloud_repository(&machine.storage.id, &invitation)
        .await
        .unwrap();
    let refused = service
        .connect_cloud_repository(
            &scope,
            &machine.storage.id,
            &resource,
            &theirs,
            saved_link_revision(&machine, &scope),
        )
        .await
        .unwrap_err();
    assert_eq!(refused, super::cloud::CLOUD_BINDING_FIXED);

    // The first Push writes into the repository Create made; the saved one
    // wins over the share the account also belongs to.
    blocking(&machine, {
        let scope = scope.clone();
        let created = created.repository_id.clone();
        move |fixture| {
            let listed = page(fixture, &scope, Some(&fixture.storage.id)).unwrap();
            assert_eq!(listed.rows[0].state, Some(CatalogLinkState::New));
            let plan = prepare_push(fixture, &scope, &fixture.storage.id);
            let destination = &plan.destinations[&listed.rows[0].resource_id.clone().unwrap()];
            assert_eq!(destination.address, "creator/review");
            assert_eq!(destination.repository_id.as_ref(), Some(&created));
            assert!(!destination.create);
            push_all(fixture, &scope, &fixture.storage.id);
        }
    })
    .await;
    assert_eq!(owned(&stub, "creator"), 1);
    let state = stub.state.lock().unwrap();
    assert_eq!(
        state.repositories[&created.repository_id].private_revision,
        1
    );
    assert_eq!(state.repositories[&theirs].private_revision, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_push_review_names_the_repositories_it_will_create() {
    let stub = StubManagedCloud::start().await;
    let root = tempfile::tempdir().unwrap();
    let machine = managed_machine(
        root.path(),
        &stub,
        "namer",
        &[
            ("review", b"---\nname: review\n---\nFirst."),
            ("notes", b"---\nname: notes\n---\nSecond."),
        ],
    );
    let plan = blocking(&machine, |fixture| {
        let scope = skill_scope(fixture).scope.scope_id;
        link_scope(fixture, &scope, &fixture.storage.id, 3);
        prepare_push(fixture, &scope, &fixture.storage.id)
    })
    .await;
    let mut destinations = plan
        .destinations
        .values()
        .map(|destination| (destination.address.as_str(), destination.create))
        .collect::<Vec<_>>();
    destinations.sort();
    assert_eq!(
        destinations,
        [("namer/notes", true), ("namer/review", true)]
    );
    assert!(plan
        .destinations
        .values()
        .all(|destination| destination.repository_id.is_none()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cloud_addresses_resolve_case_by_case_without_throwing() {
    use super::cloud::CatalogCloudResolution;
    let stub = StubManagedCloud::start().await;
    let root = tempfile::tempdir().unwrap();
    let machine = managed_machine(root.path(), &stub, "resolver", &[]);
    let service = cloud_service(&machine);
    let resolve = |address: &'static str| {
        let service = &service;
        let storage = machine.storage.id.clone();
        async move {
            service
                .resolve_cloud_repository(&storage, address)
                .await
                .unwrap()
        }
    };
    assert!(matches!(
        resolve("https://github.com/owner/name").await,
        CatalogCloudResolution::Github
    ));
    assert!(matches!(
        resolve("https://staging.example.com/owner/name").await,
        CatalogCloudResolution::WrongService { .. }
    ));
    assert!(matches!(
        resolve("not an address").await,
        CatalogCloudResolution::Invalid
    ));
    match resolve("resolver/missing").await {
        CatalogCloudResolution::NotFound { address } => assert_eq!(address, "resolver/missing"),
        other => panic!("expected not_found, got {other:?}"),
    }
    assert!(service
        .join_cloud_repository(&machine.storage.id, "  ")
        .await
        .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_sign_in_keeps_the_rows_and_closes_push_and_pull() {
    let stub = StubManagedCloud::start().await;
    let root = tempfile::tempdir().unwrap();
    let machine = managed_machine(
        root.path(),
        &stub,
        "signed-out",
        &[("review", b"---\nname: review\n---\nLocal.")],
    );
    let token = machine
        .storage
        .provider_config
        .secrets
        .get(&token_key())
        .unwrap()
        .expose()
        .to_owned();
    stub.state.lock().unwrap().revoke_token(&token);
    blocking(&machine, |fixture| {
        let scope = skill_scope(fixture).scope.scope_id;
        link_scope(fixture, &scope, &fixture.storage.id, 3);
        let listed = page(fixture, &scope, Some(&fixture.storage.id)).unwrap();
        let status = listed.cloud_status.expect("the page says why");
        assert_eq!(
            status.state,
            mallard_infrastructure::managed_cloud::client::ManagedCloudStatusState::SignInRequired
        );
        assert_eq!(listed.rows.len(), 1, "what is on this Mac still lists");
        assert_eq!(listed.rows[0].state, None, "nothing is compared");
        assert!(prepare_push_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &scope,
            &fixture.storage.id,
            &fixture.user_home,
            4,
        )
        .is_err());
        assert!(prepare_pull_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &scope,
            &fixture.storage.id,
            BTreeMap::new(),
            &fixture.user_home,
            4,
        )
        .is_err());
    })
    .await;
}
