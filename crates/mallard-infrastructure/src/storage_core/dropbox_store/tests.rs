use super::mock::{folder, MockDropboxServer as Server};
use super::*;
use crate::dropbox_setup::mock::MockDropboxSetup;
use crate::storage_core::{connection_events, ConfiguredObjectStore, ConnectionState};
use crate::storage_providers::StorageProviderRegistry;
use crate::storage_setup_journal::MemoryProbeJournal;
fn key(suffix: &str) -> ObjectKey {
    ObjectKey::parse(format!(
        ".mallard/v1/repositories/0123456789abcdef0123456789abcdef/{suffix}"
    ))
    .unwrap()
}
/// A store over the transport mock whose access renews through the setup
/// mock's token endpoint.
fn managed_store(server: &Server, oauth: &MockDropboxSetup) -> DropboxObjectStore {
    DropboxObjectStore::with_authorization(
        "shared".into(),
        DropboxAuthorization::new(
            DropboxCredential::RefreshToken(SecretValue::parse("refresh-canary").unwrap()),
            oauth.client(),
        ),
        server.store.runtime.clone(),
        &server.store.api,
        &server.store.content,
    )
    .unwrap()
}
fn manual_config(id: &str) -> mallard_domain::StorageConfig {
    let mut secrets = mallard_domain::SecretMap::empty();
    secrets
        .insert(
            crate::storage_providers::dropbox::token_key(),
            SecretValue::parse("test-token").unwrap(),
        )
        .unwrap();
    mallard_domain::StorageConfig {
        id: mallard_domain::StorageId::parse(id).unwrap(),
        name: "Dropbox".into(),
        provider: crate::storage_providers::dropbox::provider_key(),
        provider_config: mallard_domain::StorageProviderConfig::new(
            1,
            mallard_domain::BoundedConfigObject::encode(
                &crate::storage_providers::dropbox::DropboxSettingsV1 {
                    namespace_id: "shared".into(),
                    folder_name: None,
                },
            )
            .unwrap(),
            secrets,
        ),
    }
}

#[test]
fn expired_managed_token_refreshes_once_and_retries_reads_only() {
    let server = Server::new();
    let oauth = MockDropboxSetup::new();
    let store = managed_store(&server, &oauth);
    let object = key("_objects/blob");
    store.put_immutable(&object, b"blob").unwrap();
    assert_eq!(oauth.count("refresh"), 1);
    assert_eq!(
        store.connection_health().unwrap().state,
        ConnectionState::Connected
    );
    // The token's four hours ran out under a cached grant: the read is sent
    // again after one refresh, and nothing about the credential is recorded.
    server.state.lock().unwrap().expire_once = true;
    assert_eq!(
        store.get_metadata(&object, 64).unwrap().unwrap().bytes,
        b"blob"
    );
    assert_eq!(oauth.count("refresh"), 2);
    assert_eq!(server.state.lock().unwrap().routes["files/download"], 2);
    assert_eq!(
        store.connection_health().unwrap().state,
        ConnectionState::Connected
    );
    // A write is never sent twice: the grant is dropped, the caller reruns.
    server.state.lock().unwrap().expire_once = true;
    let uploads = server.state.lock().unwrap().uploads;
    let error = store
        .put_immutable(&key("_objects/second"), b"second")
        .unwrap_err();
    assert!(error.contains("expired during the request"), "{error}");
    assert!(!error.contains("summary-canary"));
    assert_eq!(oauth.count("refresh"), 2);
    assert_eq!(server.state.lock().unwrap().uploads, uploads);
    assert_eq!(
        store.connection_health().unwrap().state,
        ConnectionState::Connected
    );
    // The next request refreshes before it is sent.
    store.get_metadata(&object, 64).unwrap();
    assert_eq!(oauth.count("refresh"), 3);
    // A token Dropbox still calls expired after a refresh is a rejection.
    server.state.lock().unwrap().auth_tag = Some("expired_access_token");
    let error = store.get_metadata(&object, 64).unwrap_err();
    assert!(
        error.contains("rejected the saved authorization"),
        "{error}"
    );
    assert!(error.contains("(files/download, HTTP 401, expired_access_token, request req-"));
    assert_eq!(oauth.count("refresh"), 4);
    let health = store.connection_health().unwrap();
    assert_eq!(health.state, ConnectionState::ReconnectRequired);
    assert_eq!(health.code, "dropbox_reconnect_required");
}

#[test]
fn rejected_token_records_evidence_and_the_shared_store_stops_asking() {
    let server = Server::new();
    let registry = StorageProviderRegistry::from_adapters(vec![server.adapter()]).unwrap();
    let config = manual_config("storage-dropbox-rejected");
    // The registry opens remote stores on the caller's runtime, as the
    // desktop commands do.
    let _runtime = server.store.runtime.enter();
    let store = ConfiguredObjectStore::open_with(&registry, &config).unwrap();
    let mut events = connection_events().subscribe();
    let object = key("_objects/blob");
    store.put_immutable(&object, b"blob").unwrap();
    assert_eq!(
        store.connection_health().unwrap().state,
        ConnectionState::Connected
    );
    server.state.lock().unwrap().auth_tag = Some("invalid_access_token");
    let error = store.get_metadata(&object, 64).unwrap_err();
    assert!(
        error.starts_with("Dropbox rejected the saved access token."),
        "{error}"
    );
    assert!(error.contains("(files/download, HTTP 401, invalid_access_token, request req-"));
    assert!(!error.contains("summary-canary") && !error.contains("test-token"));
    let health = store.connection_health().unwrap();
    assert_eq!(health.state, ConnectionState::ReconnectRequired);
    assert_eq!(health.code, "dropbox_reconnect_required");
    assert!(health.suspends());
    let evidence = health.evidence.unwrap();
    assert_eq!(evidence.endpoint, "files/download");
    assert_eq!(evidence.http_status, Some(401));
    assert_eq!(evidence.error_tag, Some("invalid_access_token"));
    assert!(evidence.request_id.unwrap().starts_with("req-"));
    // Suspended: every call answers from the record and sends nothing, even
    // after Dropbox would accept the token again.
    let requests = server.state.lock().unwrap().requests;
    server.state.lock().unwrap().auth_tag = None;
    assert_eq!(store.get_metadata(&object, 64).unwrap_err(), error);
    assert_eq!(store.put_immutable(&object, b"blob").unwrap_err(), error);
    assert_eq!(
        store
            .list(
                &ObjectPrefix::parse(".mallard/v1/repositories").unwrap(),
                None,
                10
            )
            .unwrap_err(),
        error
    );
    assert_eq!(server.state.lock().unwrap().requests, requests);
    // Both transitions were announced for this storage: connected, then
    // reconnect required.
    let mut announced = Vec::new();
    while let Ok(storage_id) = events.try_recv() {
        if storage_id == config.id {
            announced.push(storage_id);
        }
    }
    assert_eq!(announced.len(), 2);
}

#[test]
fn permission_and_manual_expiry_classify_apart_from_rejection() {
    let server = Server::new();
    let store = &server.store;
    let object = key("_objects/blob");
    store.put_immutable(&object, b"blob").unwrap();
    server.state.lock().unwrap().auth_tag = Some("missing_scope");
    let error = store.get_metadata(&object, 64).unwrap_err();
    assert!(
        error.starts_with("Dropbox denied access for this connection."),
        "{error}"
    );
    assert!(error.contains("missing_scope"));
    let health = store.connection_health().unwrap();
    assert_eq!(health.state, ConnectionState::PermissionDenied);
    assert_eq!(health.code, "dropbox_permission_denied");
    // A manual token that ran out is not retried: there is nothing to
    // refresh with, and only a replacement recovers.
    server.state.lock().unwrap().auth_tag = Some("expired_access_token");
    let requests = server.state.lock().unwrap().requests;
    let error = store.get_metadata(&object, 64).unwrap_err();
    assert!(
        error.starts_with("The Dropbox access token expired."),
        "{error}"
    );
    assert_eq!(server.state.lock().unwrap().requests, requests + 1);
    let health = store.connection_health().unwrap();
    assert_eq!(health.state, ConnectionState::TokenExpired);
    assert_eq!(health.code, "dropbox_token_expired");
    // Success clears the record.
    server.state.lock().unwrap().auth_tag = None;
    store.get_metadata(&object, 64).unwrap();
    assert_eq!(
        store.connection_health().unwrap().state,
        ConnectionState::Connected
    );
    // Probe failures carry their cause as a code.
    for (error, code) in [
        (Error::Auth, "dropbox_reconnect_required"),
        (Error::Expired, "dropbox_token_expired"),
        (Error::Permission, "dropbox_permission_denied"),
        (Error::Namespace, "dropbox_namespace_rejected"),
        (Error::Unreachable, "dropbox_unreachable"),
        (Error::Unknown, "dropbox_request_failed"),
        (Error::RateLimit, "dropbox_rate_limited"),
        (Error::Cleanup, "dropbox_probe_cleanup_required"),
        (Error::Invalid, "dropbox_probe_failed"),
    ] {
        assert_eq!(store.adapter_error(error).code(), code);
    }
    assert_eq!(
        store.adapter_error(Error::Cleanup).kind(),
        crate::storage_providers::StorageAdapterErrorKind::CleanupRequired
    );
}

#[test]
fn refreshed_credential_supports_probe_and_verified_transport_without_mutation_replay() {
    let server = Server::new();
    let oauth = crate::dropbox_setup::mock::MockDropboxSetup::new();
    let store = DropboxObjectStore::with_authorization(
        "shared".into(),
        DropboxAuthorization::new(
            DropboxCredential::RefreshToken(SecretValue::parse("refresh-canary").unwrap()),
            oauth.client(),
        ),
        server.store.runtime.clone(),
        &server.store.api,
        &server.store.content,
    )
    .unwrap();
    store
        .probe(
            "setup-111111111111111111111111",
            &mut MemoryProbeJournal::default(),
        )
        .unwrap();
    let object = key("_objects/refresh-proof");
    assert_eq!(
        store.put_immutable(&object, b"proof").unwrap(),
        ImmutablePutOutcome::Written
    );
    assert!(matches!(
        store
            .stage_verified(
                &object,
                &ExpectedObject {
                    size: 5,
                    sha256: sha256_hex(b"proof")
                }
            )
            .unwrap(),
        StageOutcome::Verified(_)
    ));
    assert_eq!(oauth.count("refresh"), 1);
    server.state.lock().unwrap().fail_upload = true;
    let before = server.state.lock().unwrap().uploads;
    assert!(store
        .compare_and_swap(&key("_head.json"), &CasExpectation::Absent, b"head")
        .is_err());
    assert_eq!(server.state.lock().unwrap().uploads, before + 1);
}
#[test]
fn immutable_cas_integrity_and_conditional_delete() {
    let server = Server::new();
    let key = key("_head.json");
    let store = &server.store;
    assert!(store.get_metadata(&key, 64).unwrap().is_none());
    assert_eq!(
        store.put_immutable(&key, b"a").unwrap(),
        ImmutablePutOutcome::Written
    );
    assert_eq!(
        store.put_immutable(&key, b"a").unwrap(),
        ImmutablePutOutcome::AlreadyPresent
    );
    assert!(store.put_immutable(&key, b"b").is_err());
    let first = store.get_metadata(&key, 64).unwrap().unwrap();
    let CasOutcome::Written { etag } = store
        .compare_and_swap(&key, &CasExpectation::Match(first.etag.clone()), b"b")
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(
        store
            .compare_and_swap(&key, &CasExpectation::Match(first.etag.clone()), b"b")
            .unwrap(),
        CasOutcome::Conflict { .. }
    ));
    assert!(matches!(
        store.delete(&key, &first.etag).unwrap(),
        DeleteOutcome::Conflict { .. }
    ));
    assert!(matches!(
        store
            .stage_verified(
                &key,
                &ExpectedObject {
                    size: 1,
                    sha256: sha256_hex(b"b")
                }
            )
            .unwrap(),
        StageOutcome::Verified(_)
    ));
    server.state.lock().unwrap().bad_hash = true;
    assert!(store
        .get_metadata(&key, 64)
        .unwrap_err()
        .contains("integrity"));
    server.state.lock().unwrap().bad_hash = false;
    assert_eq!(store.delete(&key, &etag).unwrap(), DeleteOutcome::Deleted);
    assert!(matches!(
        store
            .compare_and_swap(&key, &CasExpectation::Match(etag), b"b")
            .unwrap(),
        CasOutcome::Conflict { current_etag: None }
    ));
}
#[test]
fn unknown_cas_never_infers_a_win_but_immutable_can_verify_content() {
    let server = Server::new();
    let key = key("_head.json");
    server.state.lock().unwrap().fail_upload = true;
    let error = server
        .store
        .compare_and_swap(&key, &CasExpectation::Absent, b"a")
        .unwrap_err();
    assert!(error.contains("unknown"));
    assert!(!error.contains("private_response"));
    assert_eq!(server.state.lock().unwrap().uploads, 1);
    server.state.lock().unwrap().fail_upload = true;
    assert_eq!(
        server
            .store
            .put_immutable(&super::tests::key("_objects/blob"), b"b")
            .unwrap(),
        ImmutablePutOutcome::AlreadyPresent
    );
}
#[test]
fn upload_boundary_and_batch_preflight_are_dropbox_only() {
    assert!(upload_size(DROPBOX_UPLOAD_LIMIT).is_ok());
    assert_eq!(
        upload_size(DROPBOX_UPLOAD_LIMIT + 1),
        Err(Error::UploadLimit)
    );
    let server = Server::new();
    let too_large = Bytes::from(vec![0; DROPBOX_UPLOAD_LIMIT as usize + 1]);
    assert!(server
        .store
        .put_many(vec![
            (key("_objects/small"), Bytes::from_static(b"a")),
            (key("_objects/large"), too_large)
        ])
        .unwrap_err()
        .contains("150 MiB"));
    assert_eq!(server.state.lock().unwrap().requests, 0);
}
#[test]
fn list_accepts_requested_folder_and_returns_only_child_files() {
    let server = Server::new();
    let prefix = ObjectPrefix::parse(".mallard/v1/repositories/_tags").unwrap();
    let path = format!("/{}", prefix.as_str());
    server
        .state
        .lock()
        .unwrap()
        .entries
        .insert(path.clone(), (folder(&path), vec![]));

    let empty = server.store.list(&prefix, None, 10).unwrap();
    assert!(empty.entries.is_empty());
    assert!(empty.next_cursor.is_none());

    let tag = ObjectKey::parse(format!(
        "{}/0123456789abcdef0123456789abcdef.json",
        prefix.as_str()
    ))
    .unwrap();
    server.store.put_immutable(&tag, b"tag").unwrap();
    let page = server.store.list(&prefix, None, 10).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].key, tag);
    assert!(page.next_cursor.is_none());
}
#[test]
fn list_rejects_invalid_requested_folder_and_outside_entries() {
    for fault in ["file", "name", "outside", "duplicate"] {
        let server = Server::new();
        let object = key("_objects/a");
        let prefix = ObjectPrefix::parse(key("_objects").as_str()).unwrap();
        let path = format!("/{}", prefix.as_str());
        server.store.put_immutable(&object, b"a").unwrap();
        {
            let mut state = server.state.lock().unwrap();
            let mut root = folder(&path);
            match fault {
                "file" => {
                    root = state.entries[&object_path(&object).unwrap()].0.clone();
                    root.path_lower = path.clone();
                    root.name = "_objects".into();
                }
                "name" => root.name = "_Objects".into(),
                "outside" => root = folder(&format!("{path}_other")),
                "duplicate" => {
                    state
                        .entries
                        .insert(format!("{path}/z"), (root.clone(), vec![]));
                }
                _ => unreachable!(),
            }
            state.entries.insert(path, (root, vec![]));
        }
        assert_eq!(
            server.store.list(&prefix, None, 10).unwrap_err(),
            Error::Invalid.message(),
            "{fault}"
        );
        if fault == "duplicate" {
            assert_eq!(
                server.state.lock().unwrap().routes["files/list_folder/continue"],
                1
            );
        }
    }
}
#[test]
fn list_follows_all_provider_pages_and_returns_lexical_cursor() {
    let server = Server::new();
    for suffix in ["z", "b", "a"] {
        server
            .store
            .put_immutable(&key(&format!("_objects/{suffix}")), b"a")
            .unwrap();
    }
    let prefix =
        ObjectPrefix::parse(".mallard/v1/repositories/0123456789abcdef0123456789abcdef/_objects")
            .unwrap();
    let first = server.store.list(&prefix, None, 2).unwrap();
    assert_eq!(first.entries.len(), 2);
    assert!(first.entries[0].key.as_str().ends_with("/a"));
    assert!(first.entries[1].key.as_str().ends_with("/b"));
    let last = server
        .store
        .list(&prefix, first.next_cursor.as_deref(), 2)
        .unwrap();
    assert_eq!(last.entries.len(), 1);
    assert!(last.entries[0].key.as_str().ends_with("/z"));
    assert!(last.next_cursor.is_none());
}
#[test]
fn noncanonical_keys_and_credential_errors_fail_closed() {
    let server = Server::new();
    assert!(server
        .store
        .put_immutable(&key("_objects/Upper"), b"a")
        .is_err());
    assert_eq!(server.state.lock().unwrap().requests, 0);
    server.state.lock().unwrap().deny = true;
    let error = server
        .store
        .get_metadata(&key("_head.json"), 64)
        .unwrap_err();
    assert!(error.contains("token"));
    assert!(!error.contains("must never escape"));
}
#[test]
fn probe_cleans_known_revisions_and_preserves_unknown_writes() {
    let server = Server::new();
    let id = "setup-0123456789abcdef01234567";
    let mut journal = MemoryProbeJournal::default();
    server.store.probe(id, &mut journal).unwrap();
    assert!(server
        .state
        .lock()
        .unwrap()
        .entries
        .values()
        .all(|(m, _)| m.kind == "folder"));
    server.state.lock().unwrap().fail_upload = true;
    assert_eq!(server.store.probe(id, &mut journal), Err(Error::Cleanup));
    assert_eq!(
        server.store.reconcile_probe(id, &mut journal),
        Err(Error::Cleanup)
    );
    assert!(server
        .state
        .lock()
        .unwrap()
        .entries
        .values()
        .any(|(m, _)| m.kind == "file"));
}
#[test]
fn probe_rejects_account_home_before_remote_mutation() {
    let server = Server::new();
    server.state.lock().unwrap().home = true;
    assert_eq!(
        server.store.probe(
            "setup-0123456789abcdef01234567",
            &mut MemoryProbeJournal::default()
        ),
        Err(Error::Namespace)
    );
    assert!(server.state.lock().unwrap().entries.is_empty());
}
#[test]
fn content_hash_handles_empty_and_cross_block_streams() {
    assert_eq!(content_hash(b""), sha256_hex(b""));
    let bytes = vec![17; CONTENT_BLOCK + 19];
    let mut streamed = ContentHash::default();
    for chunk in bytes.chunks(7_111) {
        streamed.update(chunk);
    }
    let mut expected = Sha256::new();
    expected.update(Sha256::digest(&bytes[..CONTENT_BLOCK]));
    expected.update(Sha256::digest(&bytes[CONTENT_BLOCK..]));
    assert_eq!(streamed.finish(), hex_lower(&expected.finalize()));
    assert_eq!(content_hash(&bytes), {
        let mut h = ContentHash::default();
        h.update(&bytes);
        h.finish()
    });
}

#[test]
fn api_error_unions_use_documented_shapes_and_never_remote_messages() {
    for error in [
        json!({".tag":"path","reason":{".tag":"conflict","conflict":{".tag":"file"}},"upload_session_id":"unused"}),
        json!({".tag":"path_write","path_write":{".tag":"conflict","conflict":{".tag":"file"}}}),
    ] {
        assert_eq!(classify_conflict(&error), Error::Conflict);
    }
    assert_eq!(
        classify_conflict(&json!({".tag":"path_lookup","path_lookup":{".tag":"not_found"}})),
        Error::Missing
    );
    assert_eq!(
        classify_conflict(
            &json!({".tag":"path","path":{".tag":"conflict","conflict":{".tag":"file_ancestor"}}})
        ),
        Error::Invalid
    );
    assert_eq!(
        classify_conflict(&json!({".tag":"path","reason":{".tag":"no_write_permission"}})),
        Error::Permission
    );
    assert_eq!(
        classify_conflict(&json!({".tag":"payload_too_large"})),
        Error::UploadLimit
    );
    assert_eq!(
        classify_conflict(&json!({".tag":"content_hash_mismatch"})),
        Error::Integrity
    );
    assert_eq!(
        classify_conflict(&json!({"error_summary":"conflict/token-secret"})),
        Error::Permission
    );
}
#[test]
fn managed_clear_keeps_marker_folders_and_unmanaged_files() {
    let server = Server::new();
    super::super::ensure_storage_marker(&server.store).unwrap();
    server
        .store
        .put_immutable(&key("_objects/blob"), b"project")
        .unwrap();
    let library = ObjectKey::parse(
        ".mallard/v1/library/library-0123456789abcdef0123456789abcdef/_objects/blob",
    )
    .unwrap();
    server.store.put_immutable(&library, b"library").unwrap();
    let result = server.store.clear_managed_data().unwrap();
    assert_eq!(result.repository_objects_deleted, 1);
    assert_eq!(result.library_objects_deleted, 1);
    assert!(server
        .store
        .get_metadata(
            &ObjectKey::parse(super::super::STORAGE_MARKER_KEY).unwrap(),
            4096
        )
        .unwrap()
        .is_some());
    assert!(server
        .state
        .lock()
        .unwrap()
        .entries
        .values()
        .any(|(m, _)| m.kind == "folder"));
}
#[test]
fn recovery_uses_the_durable_revision_after_restart() {
    use crate::persistence::LocalStateRepository;
    use crate::storage_providers::traits::StorageProbeJournal;
    use crate::storage_setup_journal::{
        ProbeJournal, StorageSetupId, StorageSetupJournalEntry, StorageSetupJournalStore,
        StorageSetupStage,
    };
    use mallard_domain::BoundedConfigObject;
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let disk = StorageSetupJournalStore::new(&repository);
    let id = "setup-0123456789abcdef01234567";
    let path = format!("/.mallard/v1/_setup_probes/{id}.bin");
    let server = Server::new();
    let metadata = server
        .store
        .runtime
        .block_on(
            server
                .store
                .upload(&path, json!({".tag":"add"}), Bytes::from_static(b"known")),
        )
        .unwrap();
    let mut entry = StorageSetupJournalEntry {
        schema: 1,
        setup_id: StorageSetupId::parse(id).unwrap(),
        provider: crate::storage_providers::dropbox::provider_key(),
        config_version: 1,
        stage: StorageSetupStage::ProbeRunning,
        error_code: None,
        created_at: 1,
        updated_at: 1,
        facts: BoundedConfigObject::from_json(&json!({"namespace_id":"shared"})).unwrap(),
        recovery: BoundedConfigObject::empty(),
    };
    ProbeJournal {
        store: &disk,
        entry: &mut entry,
    }
    .checkpoint(
        BoundedConfigObject::from_json(
            &json!({"revision":metadata.rev,"size":5,"sha256":sha256_hex(b"known")}),
        )
        .unwrap(),
    )
    .unwrap();
    let mut reloaded = disk.load(&entry.setup_id).unwrap().unwrap();
    server
        .store
        .reconcile_probe(
            id,
            &mut ProbeJournal {
                store: &disk,
                entry: &mut reloaded,
            },
        )
        .unwrap();
    assert!(!server.state.lock().unwrap().entries.contains_key(&path));
    assert!(!format!("{reloaded:?}").contains("test-token"));
}

#[test]
fn content_routes_reject_wrong_types_and_symlinks() {
    let server = Server::new();
    let key = key("_objects/file");
    server.store.put_immutable(&key, b"file").unwrap();
    let path = object_path(&key).unwrap();
    let mut metadata =
        serde_json::to_value(&server.state.lock().unwrap().entries[&path].0).unwrap();
    metadata[".tag"] = json!("folder");
    assert!(Metadata::from_file_response(&serde_json::to_vec(&metadata).unwrap()).is_err());
    server
        .state
        .lock()
        .unwrap()
        .entries
        .get_mut(&path)
        .unwrap()
        .0
        .symlink_info = Some(json!({"target":"elsewhere"}));
    assert!(server.store.get_metadata(&key, 64).is_err());
}

#[test]
fn redirects_and_rate_limits_do_not_trigger_automatic_requests() {
    for (status, expected) in [(307, "unknown"), (429, "rate limit"), (403, "denied")] {
        let server = Server::new();
        server.state.lock().unwrap().response_status = Some(status);
        let error = server
            .store
            .get_metadata(&key("_head.json"), 64)
            .unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("must never escape"));
        assert_eq!(server.state.lock().unwrap().requests, 1);
    }
}

fn upload_objects(count: usize) -> Vec<(ObjectKey, Bytes)> {
    (0..count)
        .map(|index| {
            (
                key(&format!("_objects/{index:04}")),
                Bytes::from(format!("body-{index}")),
            )
        })
        .collect()
}

fn put_at(
    server: &Server,
    objects: Vec<(ObjectKey, Bytes)>,
    concurrency: usize,
) -> Result<Vec<ImmutablePutOutcome>> {
    server
        .store
        .runtime
        .block_on(server.store.put_many_async(objects, concurrency))
}

#[test]
fn upload_windows_run_concurrently_at_the_bound_and_keep_input_order() {
    for concurrency in [1, 2, 6, 16] {
        let server = Server::new();
        server.state.lock().unwrap().start_barrier =
            Some(Arc::new(tokio::sync::Barrier::new(concurrency)));
        let objects = upload_objects(concurrency * 2);
        let outcomes = put_at(&server, objects.clone(), concurrency).unwrap();
        assert_eq!(outcomes, vec![ImmutablePutOutcome::Written; objects.len()]);
        let state = server.state.lock().unwrap();
        assert_eq!(state.max_active_starts, concurrency);
        assert_eq!(state.active_starts, 0);
        assert_eq!(state.routes["files/upload_session/start"], concurrency * 2);
        assert_eq!(state.routes["files/upload_session/finish_batch_v2"], 2);
        assert!(!state.routes.contains_key("files/upload"));
        for (key, bytes) in &objects {
            assert_eq!(state.entries[&object_path(key).unwrap()].1, bytes.as_ref());
        }
    }
}

#[test]
fn thirty_nine_payloads_use_forty_six_requests_and_no_folder_probes() {
    let server = Server::new();
    put_at(&server, upload_objects(39), 6).unwrap();
    let state = server.state.lock().unwrap();
    assert!(!state.routes.contains_key("files/get_metadata"));
    assert!(!state.routes.contains_key("files/create_folder_v2"));
    assert_eq!(state.routes["files/upload_session/start"], 39);
    assert_eq!(state.routes["files/upload_session/finish_batch_v2"], 7);
    assert_eq!(state.requests, 46);
    for path in [
        "/.mallard",
        "/.mallard/v1",
        "/.mallard/v1/repositories",
        "/.mallard/v1/repositories/0123456789abcdef0123456789abcdef",
        "/.mallard/v1/repositories/0123456789abcdef0123456789abcdef/_objects",
    ] {
        assert_eq!(state.entries[path].0.kind, "folder", "{path}");
    }
}

#[test]
fn one_window_commits_mixed_parents_without_folder_requests() {
    let server = Server::new();
    let objects = vec![
        (key("_objects/one"), Bytes::from_static(b"one")),
        (key("_commits/two"), Bytes::from_static(b"two")),
    ];
    put_at(&server, objects, 6).unwrap();
    let state = server.state.lock().unwrap();
    assert!(!state.routes.contains_key("files/create_folder_v2"));
    assert!(!state.routes.contains_key("files/get_metadata"));
    assert_eq!(state.routes["files/upload_session/start"], 2);
    assert_eq!(state.routes["files/upload_session/finish_batch_v2"], 1);
    assert_eq!(state.requests, 3);
    let repository = "/.mallard/v1/repositories/0123456789abcdef0123456789abcdef";
    assert_eq!(
        state.entries[&format!("{repository}/_objects")].0.kind,
        "folder"
    );
    assert_eq!(
        state.entries[&format!("{repository}/_commits")].0.kind,
        "folder"
    );
}

#[test]
fn preflight_covers_later_windows_empty_batches_modes_and_duplicate_keys() {
    let server = Server::new();
    assert!(put_at(&server, vec![], 6).unwrap().is_empty());
    let mut objects = upload_objects(8);
    objects.push((key("_objects/Upper"), Bytes::from_static(b"bad")));
    assert_eq!(put_at(&server, objects, 6), Err(Error::Invalid));
    for mode in [
        json!({".tag":"overwrite"}),
        json!({".tag":"update","update":"invalid"}),
        json!({".tag":"add","extra":true}),
    ] {
        assert!(matches!(
            server.store.runtime.block_on(server.store.upload(
                &object_path(&key("_objects/a")).unwrap(),
                mode,
                Bytes::new()
            )),
            Err(Error::Invalid)
        ));
    }
    assert_eq!(put_at(&server, vec![], 0), Err(Error::Limit));
    assert_eq!(put_at(&server, vec![], 17), Err(Error::Limit));
    let duplicate = key("_objects/a");
    assert_eq!(
        put_at(
            &server,
            vec![
                (duplicate.clone(), Bytes::from_static(b"a")),
                (duplicate.clone(), Bytes::from_static(b"b")),
            ],
            6
        ),
        Err(Error::Integrity)
    );
    assert_eq!(server.state.lock().unwrap().requests, 0);
    let objects = vec![
        (duplicate.clone(), Bytes::from_static(b"a")),
        (key("_objects/b"), Bytes::new()),
        (duplicate, Bytes::from_static(b"a")),
    ];
    assert_eq!(
        put_at(&server, objects.clone(), 1).unwrap(),
        vec![
            ImmutablePutOutcome::Written,
            ImmutablePutOutcome::Written,
            ImmutablePutOutcome::AlreadyPresent
        ]
    );
    assert_eq!(
        server.state.lock().unwrap().routes["files/upload_session/start"],
        2
    );
    assert_eq!(
        put_at(&server, objects, 6).unwrap(),
        vec![ImmutablePutOutcome::AlreadyPresent; 3]
    );
}

#[test]
fn a_file_where_a_folder_belongs_fails_closed_without_folder_probes() {
    let server = Server::new();
    put_at(&server, upload_objects(1), 6).unwrap();
    {
        let mut state = server.state.lock().unwrap();
        let mut blocker = folder("/.mallard/v1");
        blocker.kind = "file".into();
        state
            .entries
            .insert("/.mallard/v1".into(), (blocker, vec![]));
    }
    assert_eq!(put_at(&server, upload_objects(2), 6), Err(Error::Invalid));
    let object = key("_objects/0000");
    assert!(server.store.get_metadata(&object, 64).unwrap().is_none());
    assert!(matches!(
        server
            .store
            .stage_verified(
                &object,
                &ExpectedObject {
                    size: 6,
                    sha256: sha256_hex(b"body-0")
                }
            )
            .unwrap(),
        StageOutcome::Missing
    ));
    let state = server.state.lock().unwrap();
    assert!(!state.routes.contains_key("files/get_metadata"));
    assert!(!state.routes.contains_key("files/create_folder_v2"));
    assert!(!state
        .entries
        .contains_key(&object_path(&key("_objects/0001")).unwrap()));
}

#[test]
fn reads_listing_and_deletion_cost_one_request_each() {
    let server = Server::new();
    let head = key("_head.json");
    let CasOutcome::Written { etag } = server
        .store
        .compare_and_swap(&head, &CasExpectation::Absent, b"head")
        .unwrap()
    else {
        panic!()
    };
    let requests = || server.state.lock().unwrap().requests;
    let before = requests();
    assert_eq!(
        server.store.get_metadata(&head, 64).unwrap().unwrap().bytes,
        b"head"
    );
    assert_eq!(requests(), before + 1);
    assert!(matches!(
        server
            .store
            .stage_verified(
                &head,
                &ExpectedObject {
                    size: 4,
                    sha256: sha256_hex(b"head")
                }
            )
            .unwrap(),
        StageOutcome::Verified(_)
    ));
    assert_eq!(requests(), before + 2);
    let prefix =
        ObjectPrefix::parse(".mallard/v1/repositories/0123456789abcdef0123456789abcdef").unwrap();
    assert_eq!(
        server.store.list(&prefix, None, 10).unwrap().entries.len(),
        1
    );
    assert_eq!(requests(), before + 3);
    assert_eq!(
        server.store.delete(&head, &etag).unwrap(),
        DeleteOutcome::Deleted
    );
    assert_eq!(requests(), before + 4);
    assert!(server.store.get_metadata(&head, 64).unwrap().is_none());
    let state = server.state.lock().unwrap();
    assert!(!state.routes.contains_key("files/get_metadata"));
    assert!(!state.routes.contains_key("files/create_folder_v2"));
}

#[test]
fn bulk_reads_run_concurrently_at_the_transfer_bound_and_keep_input_order() {
    let server = Server::new();
    let objects = upload_objects(12);
    put_at(&server, objects.clone(), 6).unwrap();
    let bound = crate::app_settings::cloud_transfer_concurrency();
    server.state.lock().unwrap().download_barrier =
        Some(Arc::new(tokio::sync::Barrier::new(bound)));
    let requests: Vec<_> = objects
        .iter()
        .map(|(key, bytes)| {
            (
                key.clone(),
                ExpectedObject {
                    size: bytes.len() as u64,
                    sha256: sha256_hex(bytes),
                },
            )
        })
        .collect();
    let staged = server.store.stage_many_verified(&requests).unwrap();
    for ((_, bytes), outcome) in objects.iter().zip(&staged) {
        let StageOutcome::Verified(body) = outcome else {
            panic!("object was not staged")
        };
        assert_eq!(body.read_bounded(64, "object").unwrap(), bytes.as_ref());
    }
    let keys: Vec<_> = objects.iter().map(|(key, _)| key.clone()).collect();
    let stored = server.store.get_metadata_many(&keys, 64).unwrap();
    for ((_, bytes), stored) in objects.iter().zip(stored) {
        assert_eq!(stored.unwrap().bytes, bytes.as_ref());
    }
    {
        let mut state = server.state.lock().unwrap();
        assert_eq!(state.max_active_downloads, bound);
        assert_eq!(state.active_downloads, 0);
        assert_eq!(state.routes["files/download"], 24);
        assert!(!state.routes.contains_key("files/get_metadata"));
        state.download_barrier = None;
        state.bad_hash = true;
    }
    assert!(server
        .store
        .stage_many_verified(&requests)
        .unwrap_err()
        .contains("integrity"));
}

#[test]
fn failed_staging_stops_the_window_without_commit_or_later_uploads() {
    let server = Server::new();
    {
        let mut state = server.state.lock().unwrap();
        state.start_barrier = Some(Arc::new(tokio::sync::Barrier::new(6)));
        state.fail_start_body = Some(b"body-0".to_vec());
    }
    assert_eq!(put_at(&server, upload_objects(18), 6), Err(Error::Unknown));
    let state = server.state.lock().unwrap();
    assert_eq!(state.routes["files/upload_session/start"], 6);
    assert!(!state
        .routes
        .contains_key("files/upload_session/finish_batch_v2"));
    assert!(!state.routes.contains_key("files/get_metadata"));
    assert_eq!(state.uploads, 0);
}

#[test]
fn invalid_or_repeated_session_ids_never_reach_commit() {
    for id in [
        "".to_owned(),
        "s".repeat(16 * 1024 + 1),
        "duplicate".to_owned(),
    ] {
        let server = Server::new();
        server.state.lock().unwrap().session_id_override = Some(id);
        assert_eq!(put_at(&server, upload_objects(2), 6), Err(Error::Invalid));
        assert_eq!(server.state.lock().unwrap().uploads, 0);
    }
}

#[test]
fn malformed_commit_results_never_count_as_success() {
    for fault in [
        "count", "order", "hash", "path", "revision", "size", "symlink", "tag", "nested", "json",
    ] {
        let server = Server::new();
        server.state.lock().unwrap().finish_fault = Some(fault);
        assert!(put_at(&server, upload_objects(2), 6).is_err(), "{fault}");
        let state = server.state.lock().unwrap();
        assert_eq!(state.routes["files/upload_session/finish_batch_v2"], 1);
        assert!(!state.routes.contains_key("files/download"));
    }
}

#[test]
fn partial_commits_stop_later_windows_and_preserve_existing_head() {
    let server = Server::new();
    let head = key("_head.json");
    server
        .store
        .compare_and_swap(&head, &CasExpectation::Absent, b"old head")
        .unwrap();
    server.state.lock().unwrap().entry_failure =
        Some((1, json!({".tag":"too_many_write_operations"})));
    assert_eq!(
        put_at(&server, upload_objects(18), 6),
        Err(Error::RateLimit)
    );
    let state = server.state.lock().unwrap();
    assert_eq!(state.routes["files/upload_session/start"], 7);
    assert_eq!(state.routes["files/upload_session/finish_batch_v2"], 2);
    assert_eq!(state.entries[&object_path(&head).unwrap()].1, b"old head");
    assert!(state
        .entries
        .contains_key(&object_path(&key("_objects/0000")).unwrap()));
    assert!(!state
        .entries
        .contains_key(&object_path(&key("_objects/0001")).unwrap()));
    assert!(!state.routes.contains_key("files/delete_v2"));
}

#[test]
fn unknown_or_conflicting_commits_are_proved_by_metadata_without_body_downloads() {
    let server = Server::new();
    server.state.lock().unwrap().fail_upload = true;
    assert_eq!(
        put_at(&server, upload_objects(3), 6).unwrap(),
        vec![ImmutablePutOutcome::AlreadyPresent; 3]
    );
    {
        let state = server.state.lock().unwrap();
        assert_eq!(state.routes["files/upload_session/start"], 3);
        assert_eq!(state.routes["files/upload_session/finish_batch_v2"], 1);
        assert_eq!(state.routes["files/get_metadata"], 3);
        assert!(!state.routes.contains_key("files/download"));
    }
    // A retry after a lost commit stages every object again, receives a
    // conflict for each, and proves each by metadata.
    assert_eq!(
        put_at(&server, upload_objects(3), 6).unwrap(),
        vec![ImmutablePutOutcome::AlreadyPresent; 3]
    );
    {
        let state = server.state.lock().unwrap();
        assert_eq!(state.routes["files/get_metadata"], 6);
        assert!(!state.routes.contains_key("files/download"));
    }
    assert_eq!(
        put_at(
            &server,
            vec![(key("_objects/0000"), Bytes::from_static(b"other"))],
            6
        ),
        Err(Error::Integrity)
    );
    assert!(!server
        .state
        .lock()
        .unwrap()
        .routes
        .contains_key("files/download"));
}

#[test]
fn start_rate_limits_and_redirects_are_not_retried() {
    for (status, error) in [
        (429, Error::RateLimit),
        (307, Error::Unknown),
        (403, Error::Permission),
    ] {
        let server = Server::new();
        server.state.lock().unwrap().response_status = Some(status);
        assert_eq!(put_at(&server, upload_objects(1), 6), Err(error));
        let state = server.state.lock().unwrap();
        assert_eq!(state.requests, 1);
        assert_eq!(state.uploads, 0);
    }
}
