use super::mock::{MockFeishuServer as Server, UploadFault};
use super::*;
use crate::storage_core::{connection_events, ConfiguredObjectStore, ConnectionState};
use crate::storage_providers::StorageProviderRegistry;
use mallard_domain::sha256_hex;

const BUNDLE: &str = "0123456789abcdef0123456789abcdef";

fn key(suffix: &str) -> ObjectKey {
    ObjectKey::parse(format!(".mallard/v1/repositories/{BUNDLE}/{suffix}")).unwrap()
}
fn record_key(name: &str) -> ObjectKey {
    ObjectKey::parse(format!(".mallard/v1/repositories/_commits/{BUNDLE}/{name}")).unwrap()
}
fn digest_key(bytes: &[u8]) -> ObjectKey {
    key(&format!("_objects/{}", sha256_hex(bytes)))
}
fn record_name(revision: u64, suffix: &str) -> String {
    format!("{revision:010}.commit-{suffix}.root.0000000000001.json")
}

#[test]
fn logical_keys_are_placed_in_shard_folders_and_nothing_else_moves() {
    let object = digest_key(b"payload");
    let placed = Location::of(&object).unwrap();
    assert_eq!(
        placed.dirs,
        [
            ".mallard",
            "v1",
            "repositories",
            BUNDLE,
            "_objects",
            &sha256_hex(b"payload")[..2]
        ]
    );
    let manifest = Location::of(&key("_manifests/12345-commit-abc.json")).unwrap();
    assert_eq!(manifest.dirs[4..], ["_manifests", "0000012"]);
    let record = Location::of(&record_key(&record_name(999, "a"))).unwrap();
    assert_eq!(record.dirs[3..], ["_commits", BUNDLE, "0000000"]);
    let record = Location::of(&record_key(&record_name(1000, "a"))).unwrap();
    assert_eq!(record.dirs[3..], ["_commits", BUNDLE, "0000001"]);
    let library = ObjectKey::parse(format!(
        ".mallard/v1/library/library-{BUNDLE}/_commits/{}",
        record_name(2500, "b")
    ))
    .unwrap();
    assert_eq!(
        Location::of(&library).unwrap().dirs[4..],
        ["_commits", "0000002"]
    );
    let tag = ObjectKey::parse(format!(".mallard/v1/library/library-{BUNDLE}/_tag.json")).unwrap();
    assert_eq!(Location::of(&tag).unwrap().dirs.len(), 4);
    let marker = Location::of(&ObjectKey::parse(".mallard/_storage.json").unwrap()).unwrap();
    assert_eq!(marker.dirs, [".mallard"]);
    assert_eq!(marker.name, "_storage.json");
    // A name Mallard never writes in a family cannot be placed.
    assert_eq!(
        Location::of(&key("_objects/not-a-digest")).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        Location::of(&record_key("late.json")).unwrap_err(),
        Error::Invalid
    );
}

#[test]
fn invalid_responses_name_the_operation_without_exposing_remote_text() {
    let server = Server::new();
    let key = digest_key(b"payload");
    for (reply, expected) in [
        (
            serde_json::json!({"code":0,"data":null,"msg":"secret-canary"}),
            "Feishu response failed validation (drive/v1/files: data).",
        ),
        (
            serde_json::json!({"code":0,"msg":"secret-canary"}),
            "Feishu response failed validation (drive/v1/files: data).",
        ),
        (
            serde_json::json!({"code":0,"data":{"files":null},"msg":"secret-canary"}),
            "Feishu response failed validation (drive/v1/files: files).",
        ),
        (
            serde_json::json!({"code":1061002,"msg":"secret-canary"}),
            "Feishu rejected the request (drive/v1/files, code 1061002).",
        ),
    ] {
        let before = server.count("files");
        server.state.lock().unwrap().response_once = Some(("files", reply));
        assert_eq!(server.store.get_metadata(&key, 1024).unwrap_err(), expected);
        assert_eq!(server.count("files"), before + 1);
    }
    server.state.lock().unwrap().response_once = Some((
        "upload_all",
        serde_json::json!({"code":0,"data":{"file_token":"secret-canary/invalid"}}),
    ));
    let error = server.store.put_immutable(&key, b"payload").unwrap_err();
    assert_eq!(
        error,
        "Feishu response failed validation (drive/v1/files/upload_all: file_token)."
    );
    assert_eq!(
        server.count("upload_all"),
        1,
        "the failed write is not repeated"
    );
}

#[test]
fn json_downloads_preserve_file_bytes_and_integrity_checks() {
    let server = Server::new();
    let payloads: [&[u8]; 3] = [
        br#"{"schema":1}"#,
        br#"{"code":0,"data":{}}"#,
        br#"{"code":1061003,"msg":"file content"}"#,
    ];
    for (index, bytes) in payloads.into_iter().enumerate() {
        let object = key(&format!("_manifests/{}-commit-a.json", index + 1));
        server.store.put_immutable(&object, bytes).unwrap();
        assert_eq!(
            server
                .store
                .get_metadata(&object, 1024)
                .unwrap()
                .unwrap()
                .bytes,
            bytes
        );
        assert!(matches!(
            server
                .store
                .stage_verified(
                    &object,
                    &ExpectedObject {
                        size: bytes.len() as u64,
                        sha256: sha256_hex(bytes),
                    }
                )
                .unwrap(),
            StageOutcome::Verified(_)
        ));
        assert!(matches!(
            server
                .store
                .stage_verified(
                    &object,
                    &ExpectedObject {
                        size: bytes.len() as u64,
                        sha256: "0".repeat(64),
                    }
                )
                .unwrap(),
            StageOutcome::Mismatch
        ));
        assert!(server
            .store
            .get_metadata(&object, bytes.len() as u64 - 1)
            .is_err());
    }
}

#[test]
fn objects_records_and_listings_round_trip_through_shard_folders() {
    let server = Server::new();
    let store = &server.store;
    let payload = b"payload bytes";
    let object = digest_key(payload);
    assert!(store.get_metadata(&object, 64).unwrap().is_none());
    assert_eq!(
        store.put_immutable(&object, payload).unwrap(),
        ImmutablePutOutcome::Written
    );
    assert_eq!(
        store.put_immutable(&object, payload).unwrap(),
        ImmutablePutOutcome::AlreadyPresent
    );
    assert!(
        store.put_immutable(&object, b"other").is_err(),
        "a key holds one byte string"
    );
    let manifest = key("_manifests/1-commit-a.json");
    store.put_immutable(&manifest, b"manifest").unwrap();
    let record = record_key(&record_name(1, "a"));
    let before = server.count("upload_all");
    assert_eq!(
        store.put_record(&record, b"record").unwrap(),
        ImmutablePutOutcome::Written
    );
    assert_eq!(server.count("upload_all"), before + 1);
    let marker = ObjectKey::parse(".mallard/_storage.json").unwrap();
    store.put_immutable(&marker, b"{}").unwrap();

    let shard = &sha256_hex(payload)[..2];
    assert_eq!(
        server.file_paths(),
        vec![
            ".mallard/_storage.json".to_owned(),
            format!(".mallard/v1/repositories/{BUNDLE}/_manifests/0000000/1-commit-a.json"),
            format!(
                ".mallard/v1/repositories/{BUNDLE}/_objects/{shard}/{}",
                sha256_hex(payload)
            ),
            format!(
                ".mallard/v1/repositories/_commits/{BUNDLE}/0000000/{}",
                record_name(1, "a")
            ),
        ]
    );
    assert_eq!(
        store.get_metadata(&object, 64).unwrap().unwrap().bytes,
        payload
    );
    assert!(matches!(
        store
            .stage_verified(
                &object,
                &ExpectedObject {
                    size: payload.len() as u64,
                    sha256: sha256_hex(payload)
                }
            )
            .unwrap(),
        StageOutcome::Verified(_)
    ));
    assert!(matches!(
        store
            .stage_verified(
                &object,
                &ExpectedObject {
                    size: payload.len() as u64,
                    sha256: "0".repeat(64)
                }
            )
            .unwrap(),
        StageOutcome::Mismatch
    ));
    // Listings drop the shard component and carry the file token.
    let listed = store
        .list(
            &ObjectPrefix::parse(format!(".mallard/v1/repositories/{BUNDLE}")).unwrap(),
            None,
            10,
        )
        .unwrap();
    assert_eq!(
        listed
            .keys()
            .map(|k| k.as_str().to_owned())
            .collect::<Vec<_>>(),
        vec![manifest.as_str().to_owned(), object.as_str().to_owned()]
    );
    assert!(listed
        .entries
        .iter()
        .all(|entry| entry.etag.starts_with("node")));
    let stream =
        ObjectPrefix::parse(format!(".mallard/v1/repositories/_commits/{BUNDLE}")).unwrap();
    assert_eq!(
        store.list_keys(&stream, None, 10).unwrap().keys,
        vec![record.clone()]
    );
    // Cursor and limit walk the same sorted set.
    let page = store
        .list(
            &ObjectPrefix::parse(format!(".mallard/v1/repositories/{BUNDLE}")).unwrap(),
            None,
            1,
        )
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    let rest = store
        .list(
            &ObjectPrefix::parse(format!(".mallard/v1/repositories/{BUNDLE}")).unwrap(),
            page.next_cursor.as_deref(),
            1,
        )
        .unwrap();
    assert_eq!(rest.entries[0].key, object);
    assert!(rest.next_cursor.is_none());
    // An absent prefix lists nothing and creates nothing.
    let files = server.file_count();
    assert!(store
        .list(
            &ObjectPrefix::parse(".mallard/v1/library").unwrap(),
            None,
            10
        )
        .unwrap()
        .entries
        .is_empty());
    assert_eq!(server.file_count(), files);
}

#[test]
fn paginated_listings_are_exhausted_and_a_stream_of_many_records_shards_by_thousand() {
    let server = Server::new();
    server.state.lock().unwrap().page_size = 2;
    let store = &server.store;
    let mut names = Vec::new();
    for revision in [1u64, 2, 999, 1000, 1001, 2000] {
        let name = record_name(revision, &format!("{revision:03}"));
        store.put_record(&record_key(&name), b"r").unwrap();
        names.push(name);
    }
    assert_eq!(
        server
            .folders_at(&format!(
                ".mallard/v1/repositories/_commits/{BUNDLE}/0000000"
            ))
            .len(),
        1
    );
    assert_eq!(
        server
            .folders_at(&format!(
                ".mallard/v1/repositories/_commits/{BUNDLE}/0000002"
            ))
            .len(),
        1
    );
    let stream =
        ObjectPrefix::parse(format!(".mallard/v1/repositories/_commits/{BUNDLE}")).unwrap();
    let listed = store.list_keys(&stream, None, 100).unwrap();
    names.sort();
    assert_eq!(
        listed
            .keys
            .iter()
            .map(|k| k.as_str().rsplit('/').next().unwrap().to_owned())
            .collect::<Vec<_>>(),
        names
    );
}

#[test]
fn same_named_sibling_folders_are_one_directory_and_writes_go_to_the_smallest_token() {
    let server = Server::new();
    let store = &server.store;
    let first = b"first";
    let second = b"second";
    store.put_immutable(&digest_key(first), first).unwrap();
    // Another computer created the same `_objects` folder at once and put
    // its object there.
    let bundle_folder = server.folders_at(&format!(".mallard/v1/repositories/{BUNDLE}"))[0].clone();
    let twin = server.add_folder(&bundle_folder, "_objects");
    let shard = server.add_folder(&twin, &sha256_hex(second)[..2]);
    server.add_file(&shard, &sha256_hex(second), second);
    let other = server.second_store();
    // Reads take the union.
    assert_eq!(
        other
            .get_metadata(&digest_key(second), 64)
            .unwrap()
            .unwrap()
            .bytes,
        second
    );
    assert_eq!(
        other
            .get_metadata(&digest_key(first), 64)
            .unwrap()
            .unwrap()
            .bytes,
        first
    );
    let listed = other
        .list(
            &ObjectPrefix::parse(format!(".mallard/v1/repositories/{BUNDLE}/_objects")).unwrap(),
            None,
            10,
        )
        .unwrap();
    assert_eq!(listed.entries.len(), 2);
    // Writes go to the folder with the smallest token: the original.
    let third = b"third";
    other.put_immutable(&digest_key(third), third).unwrap();
    let objects = server.folders_at(&format!(".mallard/v1/repositories/{BUNDLE}/_objects"));
    assert_eq!(objects.len(), 2);
    let shards = server.folders_at(&format!(
        ".mallard/v1/repositories/{BUNDLE}/_objects/{}",
        &sha256_hex(third)[..2]
    ));
    assert_eq!(shards.len(), 1);
    let state = server.state.lock().unwrap();
    assert_eq!(state.nodes[&shards[0]].parent, objects[0]);
    assert!(objects[0] < objects[1]);
}

#[test]
fn duplicate_files_read_from_the_smallest_token_and_foreign_names_are_skipped() {
    let server = Server::new();
    let store = &server.store;
    let payload = b"payload";
    store.put_immutable(&digest_key(payload), payload).unwrap();
    let shard = server.folders_at(&format!(
        ".mallard/v1/repositories/{BUNDLE}/_objects/{}",
        &sha256_hex(payload)[..2]
    ))[0]
        .clone();
    // A second copy of the same name with other bytes: the smallest token
    // is read, and every read verifies the digest.
    server.add_file(&shard, &sha256_hex(payload), b"tampered");
    let other = server.second_store();
    assert_eq!(
        other
            .get_metadata(&digest_key(payload), 64)
            .unwrap()
            .unwrap()
            .bytes,
        payload
    );
    // A file dropped in by a person, an online document, a folder in the
    // wrong place: reported and skipped, never a failed listing.
    server.add_file(&shard, "Notes.docx", b"doc");
    let objects =
        server.folders_at(&format!(".mallard/v1/repositories/{BUNDLE}/_objects"))[0].clone();
    server.add_file(&objects, &sha256_hex(b"unsharded"), b"unsharded");
    let mut events = crate::storage_core::publication::ignored_name_events().subscribe();
    let listed = other
        .list(
            &ObjectPrefix::parse(format!(".mallard/v1/repositories/{BUNDLE}")).unwrap(),
            None,
            10,
        )
        .unwrap();
    assert_eq!(listed.entries.len(), 1);
    let mut reported = Vec::new();
    while let Ok(name) = events.try_recv() {
        reported.push(name);
    }
    assert!(
        reported
            .iter()
            .any(|name| name.ends_with("/_objects/Notes.docx")),
        "{reported:?}"
    );
    assert!(
        reported
            .iter()
            .any(|name| name.ends_with(&sha256_hex(b"unsharded"))),
        "{reported:?}"
    );
}

#[test]
fn a_lost_upload_answer_is_settled_by_one_listing_and_never_resent() {
    let server = Server::new();
    let store = &server.store;
    let payload = b"lost answer";
    server.state.lock().unwrap().lose_upload_answer_once = true;
    assert_eq!(
        store.put_immutable(&digest_key(payload), payload).unwrap(),
        ImmutablePutOutcome::AlreadyPresent
    );
    assert_eq!(server.count("upload_all"), 1);
    assert_eq!(server.file_count(), 1);
    // A server error before anything was stored: the listing finds nothing
    // and the request fails; nothing is sent again.
    server.state.lock().unwrap().fail_upload_once = true;
    let error = store
        .put_immutable(&digest_key(b"never stored"), b"never stored")
        .unwrap_err();
    assert!(error.contains("result is unknown"), "{error}");
    assert_eq!(server.count("upload_all"), 2);
    assert_eq!(server.file_count(), 1);
    // The next Push skips what is present and uploads the rest: one
    // listing of the folder, one upload.
    let listings = server.count("files");
    let outcomes = store
        .put_many(vec![
            (digest_key(payload), Bytes::from_static(payload)),
            (
                digest_key(b"never stored"),
                Bytes::from_static(b"never stored"),
            ),
        ])
        .unwrap();
    assert_eq!(
        outcomes,
        vec![
            ImmutablePutOutcome::AlreadyPresent,
            ImmutablePutOutcome::Written
        ]
    );
    assert_eq!(server.count("upload_all"), 3);
    // Two objects, two shard folders: one listing each, plus none for the
    // cached ancestors.
    assert_eq!(server.count("files") - listings, 2);
    // A record is one request, never retried: a failure is the caller's.
    server.state.lock().unwrap().fail_upload_once = true;
    assert!(store
        .put_record(&record_key(&record_name(1, "a")), b"record")
        .is_err());
    assert_eq!(server.count("upload_all"), 4);
}

#[test]
fn large_objects_go_through_prepare_parts_and_finish() {
    let server = Server::new();
    server.state.lock().unwrap().block_size = 1024 * 1024;
    let store = &server.store;
    let large: Vec<u8> = (0..(SMALL_UPLOAD_LIMIT as usize + 17))
        .map(|i| (i % 251) as u8)
        .collect();
    let key = digest_key(&large);
    assert_eq!(
        store.put_immutable(&key, &large).unwrap(),
        ImmutablePutOutcome::Written
    );
    assert_eq!(server.count("upload_prepare"), 1);
    assert_eq!(server.count("upload_part"), 21);
    assert_eq!(server.count("upload_finish"), 1);
    assert_eq!(server.count("upload_all"), 0);
    assert!(matches!(
        store
            .stage_verified(
                &key,
                &ExpectedObject {
                    size: large.len() as u64,
                    sha256: sha256_hex(&large)
                }
            )
            .unwrap(),
        StageOutcome::Verified(_)
    ));
    let too_large = vec![0u8; FEISHU_UPLOAD_LIMIT as usize + 1];
    let error = store
        .put_immutable(&digest_key(&too_large), &too_large)
        .unwrap_err();
    assert!(error.contains("100 MiB"), "{error}");
}

#[test]
fn multipart_accepts_success_acknowledgements_without_object_data() {
    let large = vec![7; SMALL_UPLOAD_LIMIT as usize + 17];
    let key = digest_key(&large);
    let expected = ExpectedObject {
        size: large.len() as u64,
        sha256: sha256_hex(&large),
    };
    for response in [
        r#"{"code":0,"data":null}"#,
        r#"{"code":0}"#,
        r#"{"code":0,"data":{}}"#,
    ] {
        let server = Server::new();
        {
            let mut state = server.state.lock().unwrap();
            state.block_size = SMALL_UPLOAD_LIMIT;
            state.upload_part_response = Some(response.into());
        }
        assert_eq!(
            server.store.put_immutable(&key, &large).unwrap(),
            ImmutablePutOutcome::Written,
            "{response}"
        );
        assert_eq!(server.count("upload_prepare"), 1);
        assert_eq!(server.count("upload_part"), 2);
        assert_eq!(server.count("upload_finish"), 1);
        assert_eq!(server.count("upload_all"), 0);
        assert!(matches!(
            server.store.stage_verified(&key, &expected).unwrap(),
            StageOutcome::Verified(_)
        ));
    }
}

#[test]
fn multipart_rejects_failed_or_invalid_acknowledgements_without_resending() {
    let large = vec![7; SMALL_UPLOAD_LIMIT as usize + 17];
    let key = digest_key(&large);
    for (response, expected) in [
        (r#"{"code":1061002,"msg":"secret-canary"}"#, Error::Unknown),
        (
            r#"{"data":null,"msg":"secret-canary"}"#,
            Error::InvalidResponse("drive/v1/files/upload_part", "API error code"),
        ),
        (
            r#"{"code":"0","msg":"secret-canary"}"#,
            Error::InvalidResponse("drive/v1/files/upload_part", "API error code"),
        ),
        (
            "not-json-secret-canary",
            Error::InvalidResponse("drive/v1/files/upload_part", "JSON body"),
        ),
    ] {
        let server = Server::new();
        {
            let mut state = server.state.lock().unwrap();
            state.block_size = SMALL_UPLOAD_LIMIT;
            state.upload_part_response = Some(response.into());
        }
        assert_eq!(
            server.store.put_immutable(&key, &large).unwrap_err(),
            String::from(expected)
        );
        assert_eq!(server.count("upload_prepare"), 1);
        assert_eq!(server.count("upload_part"), 1, "the part is not resent");
        assert_eq!(server.count("upload_finish"), 0);
        assert_eq!(server.count("upload_all"), 0);
        assert!(server.state.lock().unwrap().upload_bodies.is_empty());
    }
}

#[test]
fn multipart_prepare_and_finish_still_require_their_payloads() {
    let large = vec![7; SMALL_UPLOAD_LIMIT as usize + 17];
    let key = digest_key(&large);
    for (route, response, field) in [
        (
            "upload_prepare",
            serde_json::json!({"code":0,"data":null}),
            "data",
        ),
        ("upload_prepare", serde_json::json!({"code":0}), "data"),
        (
            "upload_prepare",
            serde_json::json!({"code":0,"data":{}}),
            "upload_id",
        ),
        (
            "upload_prepare",
            serde_json::json!({"code":0,"data":{"upload_id":"upload1"}}),
            "block_size",
        ),
        (
            "upload_prepare",
            serde_json::json!({"code":0,"data":{"upload_id":"upload1","block_size":4194304}}),
            "block_num",
        ),
        (
            "upload_finish",
            serde_json::json!({"code":0,"data":null}),
            "data",
        ),
        ("upload_finish", serde_json::json!({"code":0}), "data"),
        (
            "upload_finish",
            serde_json::json!({"code":0,"data":{}}),
            "file_token",
        ),
    ] {
        let server = Server::new();
        {
            let mut state = server.state.lock().unwrap();
            state.block_size = SMALL_UPLOAD_LIMIT;
            state.response_once = Some((route, response));
        }
        assert_eq!(
            server.store.put_immutable(&key, &large).unwrap_err(),
            format!("Feishu response failed validation (drive/v1/files/{route}: {field}).")
        );
        assert_eq!(server.count("upload_prepare"), 1);
        let prepared = route == "upload_finish";
        assert_eq!(server.count("upload_part"), if prepared { 2 } else { 0 });
        assert_eq!(server.count("upload_finish"), usize::from(prepared));
        assert_eq!(server.count("upload_all"), 0);
    }
}

#[test]
fn one_request_in_flight_with_the_documented_gap() {
    let server = Server::new();
    let store = &server.store;
    store.put_immutable(&digest_key(b"gap"), b"gap").unwrap();
    let started = Instant::now();
    for _ in 0..3 {
        store.get_metadata(&digest_key(b"gap"), 64).unwrap();
    }
    // Three cached-token downloads: at least two gaps between them.
    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn an_expired_token_refreshes_once_and_repeats_reads_only() {
    let server = Server::new();
    let store = &server.store;
    let object = digest_key(b"blob");
    store.put_immutable(&object, b"blob").unwrap();
    assert_eq!(server.count("refresh"), 1);
    assert_eq!(
        store.connection_health().unwrap().state,
        ConnectionState::Connected
    );
    server.state.lock().unwrap().expire_once = true;
    assert_eq!(
        store.get_metadata(&object, 64).unwrap().unwrap().bytes,
        b"blob"
    );
    assert_eq!(server.count("refresh"), 2);
    assert_eq!(server.count("download"), 2);
    assert_eq!(
        store.connection_health().unwrap().state,
        ConnectionState::Connected
    );
    // A write is never sent twice: the grant is dropped, the caller reruns.
    server.state.lock().unwrap().expire_upload_once = true;
    let uploads = server.count("upload_all");
    let files = server.file_count();
    let error = store
        .put_immutable(&digest_key(b"second"), b"second")
        .unwrap_err();
    assert!(error.contains("expired during the request"), "{error}");
    assert!(!error.contains("fixture-msg"));
    assert_eq!(
        server.count("upload_all"),
        uploads + 1,
        "one request, never a second"
    );
    assert_eq!(server.file_count(), files);
    assert_eq!(server.count("refresh"), 2);
    store.get_metadata(&object, 64).unwrap();
    assert_eq!(server.count("refresh"), 3);
    // A token Feishu still refuses after a refresh is a rejected sign-in.
    server.state.lock().unwrap().reject_tokens = true;
    let error = store.get_metadata(&object, 64).unwrap_err();
    assert!(error.contains("rejected the saved sign-in"), "{error}");
    assert!(
        error.contains("(drive/v1/files/download, HTTP 401, 99991663, request req-fixture)"),
        "{error}"
    );
    let health = store.connection_health().unwrap();
    assert_eq!(health.state, ConnectionState::ReconnectRequired);
    assert_eq!(health.code, "feishu_reconnect_required");
}

#[test]
fn a_rejected_sign_in_suspends_the_shared_store_and_announces_transitions() {
    let server = Server::new();
    let registry = StorageProviderRegistry::from_adapters(vec![server.adapter()]).unwrap();
    let config = server.config("storage-feishu-rejected");
    let _runtime = server.runtime().enter();
    let store = ConfiguredObjectStore::open_with(&registry, &config).unwrap();
    let mut events = connection_events().subscribe();
    let object = digest_key(b"blob");
    store.put_immutable(&object, b"blob").unwrap();
    server.state.lock().unwrap().reject_tokens = true;
    let error = store.get_metadata(&object, 64).unwrap_err();
    assert!(
        error.starts_with("Feishu rejected the saved sign-in."),
        "{error}"
    );
    assert!(!error.contains("fixture-msg") && !error.contains("access-"));
    let requests = server.requests();
    server.state.lock().unwrap().reject_tokens = false;
    assert_eq!(store.get_metadata(&object, 64).unwrap_err(), error);
    assert_eq!(store.put_immutable(&object, b"blob").unwrap_err(), error);
    assert_eq!(server.requests(), requests, "suspended: nothing is sent");
    let mut announced = 0;
    while let Ok(storage_id) = events.try_recv() {
        if storage_id == config.id {
            announced += 1;
        }
    }
    assert_eq!(announced, 2);
    assert!(!store.supports_claims());
    assert!(!store.supports_conditional_delete());
    assert!(store.clear_managed_data().is_err());
}

#[test]
fn rate_limits_quota_and_probe_failures_carry_their_codes() {
    let server = Server::new();
    let store = &server.store;
    server.state.lock().unwrap().rate_limit_once = true;
    let error = store
        .put_immutable(&digest_key(b"limited"), b"limited")
        .unwrap_err();
    assert!(error.contains("rate limit"), "{error}");
    server.state.lock().unwrap().children_limit = 2;
    // The shard folder takes one file; the limit stops the second.
    let a = b"quota-a";
    store.put_immutable(&digest_key(a), a).unwrap();
    let mut b = Vec::new();
    for i in 0..10_000u32 {
        let candidate = format!("quota-{i}");
        if sha256_hex(candidate.as_bytes())[..2] == sha256_hex(a)[..2] {
            b = candidate.into_bytes();
            break;
        }
    }
    assert!(!b.is_empty());
    server.state.lock().unwrap().children_limit = 1;
    let error = store.put_immutable(&digest_key(&b), &b).unwrap_err();
    assert!(error.contains("full"), "{error}");
    for (error, code) in [
        (Error::Auth, "feishu_reconnect_required"),
        (Error::Permission, "feishu_permission_denied"),
        (Error::Unreachable, "feishu_unreachable"),
        (Error::Unknown, "feishu_request_failed"),
        (Error::RateLimit, "feishu_rate_limited"),
        (Error::Quota, "feishu_quota_reached"),
        (Error::NotSaved, "feishu_connection_not_saved"),
        (Error::Invalid, "feishu_probe_failed"),
    ] {
        assert_eq!(store.adapter_error(error).code(), code);
    }
}

#[test]
fn the_probe_leaves_one_file() {
    let server = Server::new();
    let store = &server.store;
    store.probe().unwrap();
    assert_eq!(server.requests(), 15);
    assert_eq!(server.count("refresh"), 1);
    assert_eq!(server.count("files"), 8);
    assert_eq!(server.count("create_folder"), 3);
    assert_eq!(server.count("upload_all"), 1);
    assert_eq!(server.count("download"), 1);
    assert_eq!(server.count("user_info"), 1);
    // The probe file carries a fresh generated name and stays.
    let paths = server.file_paths();
    assert_eq!(paths.len(), 1);
    let id = paths[0]
        .strip_prefix(".mallard/v1/_setup_probes/setup-")
        .and_then(|rest| rest.strip_suffix(".bin"))
        .unwrap();
    assert_eq!(id.len(), 24);
    assert!(id
        .bytes()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')));
    // A refused sign-in fails the probe with its code.
    server.state.lock().unwrap().reject_tokens = true;
    let registry = StorageProviderRegistry::from_adapters(vec![server.adapter()]).unwrap();
    let error = registry
        .require(&crate::storage_providers::feishu::provider_key(), 1)
        .unwrap()
        .probe(
            &server.config("storage-feishu-probe").provider_config,
            server.runtime(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "feishu_reconnect_required");
}

fn probe_folder(server: &Server) -> String {
    let mallard = server.add_folder(mock::ROOT, ".mallard");
    let v1 = server.add_folder(&mallard, "v1");
    server.add_folder(&v1, "_setup_probes")
}

#[test]
fn probe_existing_folders_uses_nine_requests() {
    let server = Server::new();
    probe_folder(&server);
    server.store.probe().unwrap();
    // Refresh, account, four folder reads, upload, download, listing.
    assert_eq!(server.requests(), 9);
    assert_eq!(server.count("files"), 5);
    assert_eq!(server.count("create_folder"), 0);

    // A second full test still writes, reads, and lists. Even a reused store
    // must verify root access again instead of trusting its folder cache.
    let before = server.requests();
    server.store.probe().unwrap();
    assert_eq!(server.requests() - before, 8); // Access grant is still fresh.
    assert_eq!(server.count("upload_all"), 2);
    assert_eq!(server.count("download"), 2);
}

#[test]
fn probe_consumes_root_pages_once() {
    let server = Server::new();
    server.add_folder(mock::ROOT, "unrelated-a");
    server.add_folder(mock::ROOT, "unrelated-b");
    probe_folder(&server);
    server.state.lock().unwrap().page_size = 1;
    server.store.probe().unwrap();
    // Two more root pages than the single-page walk.
    assert_eq!(server.requests(), 11);
}

#[test]
fn probe_rejects_a_refused_request_and_a_full_probe_folder_before_upload() {
    let server = Server::new();
    let folder = probe_folder(&server);
    server.state.lock().unwrap().response_status = Some(403);
    assert_eq!(server.store.probe(), Err(Error::Permission));
    assert_eq!(server.count("upload_all"), 0);

    server.state.lock().unwrap().response_status = None;
    for index in 0..MAX_FOLDER_CHILDREN {
        server.add_file(&folder, &format!("existing-{index}"), b"probe");
    }
    assert_eq!(server.store.probe(), Err(Error::Quota));
    assert_eq!(server.count("upload_all"), 0);
}

#[test]
fn probe_rejects_corrupt_missing_and_unlisted_uploads_without_resending() {
    for (fault, expected) in [
        (UploadFault::Corrupt, Error::Integrity),
        (UploadFault::Drop, Error::Integrity),
        (UploadFault::Hide, Error::Invalid),
    ] {
        let server = Server::new();
        probe_folder(&server);
        server.state.lock().unwrap().upload_fault_once = Some(fault);
        assert_eq!(server.store.probe(), Err(expected), "{fault:?}");
        assert_eq!(server.count("upload_all"), 1, "{fault:?}");
    }
}

#[test]
fn probe_reports_a_refused_refresh_and_repeats_an_expired_read_once() {
    let server = Server::new();
    probe_folder(&server);
    server.state.lock().unwrap().deny_refresh = true;
    assert_eq!(server.store.probe(), Err(Error::Auth));
    assert_eq!(server.requests(), 1);
    {
        let mut state = server.state.lock().unwrap();
        state.deny_refresh = false;
        state.expire_once = true;
    }
    let before = server.requests();
    server.store.probe().unwrap();
    // The nine requests of a fresh probe, plus the refused account read and
    // the refresh that replaced its grant.
    assert_eq!(server.requests() - before, 11);
}

#[test]
fn a_saved_configuration_opens_through_the_registry_and_rotation_never_touches_it() {
    let server = Server::new();
    let registry = StorageProviderRegistry::from_adapters(vec![server.adapter()]).unwrap();
    let config = server.config("storage-feishu-pinned");
    let _runtime = server.runtime().enter();
    let store = ConfiguredObjectStore::share_with(&registry, &config).unwrap();
    let before = serde_json::to_string(&config).unwrap();
    store.put_immutable(&digest_key(b"one"), b"one").unwrap();
    assert_eq!(server.count("refresh"), 1);
    assert_eq!(serde_json::to_string(&config).unwrap(), before);
    // The same configuration reuses the store: its cached grant and caches.
    let again = ConfiguredObjectStore::share_with(&registry, &config).unwrap();
    again.get_metadata(&digest_key(b"one"), 64).unwrap();
    assert_eq!(server.count("refresh"), 1);
    let document = server
        .connections
        .load(&server.connection_id)
        .unwrap()
        .unwrap();
    assert_ne!(document.refresh_token.expose(), Server::FIRST_REFRESH);
    assert!(!document.refresh_uncertain);
    ConfiguredObjectStore::forget(&config.id);
}

/// Request-count POC against the real adapter and a synthetic Drive tree.
/// Each object occupies a different existing shard. No user state is read.
fn lookup_poc_fixture(server: &Server, populated: bool) -> Vec<(ObjectKey, Bytes)> {
    let mut parent = super::mock::ROOT.to_owned();
    for name in [".mallard", "v1", "repositories", BUNDLE, "_objects"] {
        parent = server.add_folder(&parent, name);
    }
    let mut shards = BTreeSet::new();
    let mut objects = Vec::new();
    for index in 0.. {
        let bytes = Bytes::from(format!("lookup-poc-{index}"));
        let digest = sha256_hex(&bytes);
        if !shards.insert(digest[..2].to_owned()) {
            continue;
        }
        let shard = server.add_folder(&parent, &digest[..2]);
        if populated {
            server.add_file(&shard, &digest, &bytes);
        }
        objects.push((digest_key(&bytes), bytes));
        if objects.len() == 40 {
            return objects;
        }
    }
    unreachable!()
}

fn run_lookup_request_poc(new_shards: bool) {
    let server = Server::new();
    let objects = lookup_poc_fixture(&server, false);
    if new_shards {
        let parent =
            server.folders_at(&format!(".mallard/v1/repositories/{BUNDLE}/_objects"))[0].clone();
        server
            .state
            .lock()
            .unwrap()
            .nodes
            .retain(|_, node| node.parent != parent);
    }
    let mut sizes: Vec<u64> = objects
        .iter()
        .map(|(_, bytes)| bytes.len() as u64)
        .collect();
    sizes.sort_unstable();
    let before = server.count("files");
    let mut reported = Vec::new();
    let outcomes = server
        .store
        .put_many_with_progress(objects.clone(), &mut |size| reported.push(size))
        .unwrap();
    assert_eq!(outcomes, vec![ImmutablePutOutcome::Written; 40]);
    let lists = server.count("files") - before;
    assert_eq!(
        lists,
        if new_shards { 86 } else { 46 },
        "five ancestors and the parent once, one listing per shard, one confirmation per created shard"
    );
    assert_eq!(server.count("upload_all"), 40);
    assert_eq!(
        server.count("create_folder"),
        if new_shards { 40 } else { 0 }
    );
    assert_eq!(server.file_count(), 40);
    reported.sort_unstable();
    assert_eq!(reported, sizes, "each input is reported once");
    // The same operation again: every shard is remembered, so one listing
    // per shard settles it, nothing is uploaded, and each input is still
    // reported once.
    reported.clear();
    let outcomes = server
        .store
        .put_many_with_progress(objects.clone(), &mut |size| reported.push(size))
        .unwrap();
    assert_eq!(outcomes, vec![ImmutablePutOutcome::AlreadyPresent; 40]);
    let repeat_lists = server.count("files") - before - lists;
    assert_eq!(repeat_lists, 40);
    assert_eq!(server.count("upload_all"), 40);
    reported.sort_unstable();
    assert_eq!(reported, sizes);
    println!("REQUEST_POC new_shards={new_shards} objects=40 operations=1 lists={lists} creates={} uploads=40 repeat_lists={repeat_lists}", server.count("create_folder"));
}
#[test]
fn one_folder_is_listed_once_however_many_transfer_windows_its_objects_fill() {
    let server = Server::new();
    let mut parent = super::mock::ROOT.to_owned();
    for name in [".mallard", "v1", "repositories", BUNDLE, "_objects"] {
        parent = server.add_folder(&parent, name);
    }
    // Enough objects of one shard for several transfer windows elsewhere.
    let count = crate::app_settings::cloud_transfer_concurrency() * 2 + 1;
    let mut shard: Option<String> = None;
    let mut objects: Vec<(ObjectKey, Bytes)> = Vec::new();
    for index in 0.. {
        let bytes = Bytes::from(format!("one-folder-{index}"));
        let prefix = sha256_hex(&bytes)[..2].to_owned();
        if *shard.get_or_insert_with(|| prefix.clone()) != prefix {
            continue;
        }
        objects.push((digest_key(&bytes), bytes));
        if objects.len() == count {
            break;
        }
    }
    server.add_folder(&parent, shard.as_deref().unwrap());
    let before = server.count("files");
    let mut reported = 0;
    let outcomes = server
        .store
        .put_many_with_progress(objects.clone(), &mut |_| reported += 1)
        .unwrap();
    assert_eq!(outcomes, vec![ImmutablePutOutcome::Written; count]);
    // Five ancestors, the parent, and the shard folder, each listed once:
    // what the first listing found is carried across every upload.
    assert_eq!(server.count("files") - before, 7);
    assert_eq!(server.count("upload_all"), count);
    assert_eq!(server.count("create_folder"), 0);
    assert_eq!(server.file_count(), count);
    assert_eq!(reported, count);
}
#[test]
fn present_objects_upload_nothing_and_each_input_is_reported_once() {
    let server = Server::new();
    let mut objects = lookup_poc_fixture(&server, true);
    objects.push(objects[0].clone());
    let mut sizes: Vec<u64> = objects
        .iter()
        .map(|(_, bytes)| bytes.len() as u64)
        .collect();
    sizes.sort_unstable();
    let mut reported = Vec::new();
    let outcomes = server
        .store
        .put_many_with_progress(objects.clone(), &mut |size| reported.push(size))
        .unwrap();
    assert_eq!(outcomes, vec![ImmutablePutOutcome::AlreadyPresent; 41]);
    assert_eq!(server.count("upload_all"), 0);
    assert_eq!(server.count("create_folder"), 0);
    assert_eq!(server.file_count(), 40);
    reported.sort_unstable();
    assert_eq!(
        reported, sizes,
        "a repeated input is reported for each copy"
    );
}
#[test]
fn lookup_poc_existing_shards() {
    run_lookup_request_poc(false);
}
#[test]
fn lookup_poc_new_shards() {
    run_lookup_request_poc(true);
}
#[test]
fn batched_lookup_unions_all_parents_and_pages_without_remembering_other_siblings() {
    let server = Server::new();
    server.state.lock().unwrap().page_size = 1;
    let a = server.add_folder(super::mock::ROOT, "a");
    let twin = server.add_folder(super::mock::ROOT, "a");
    let x = server.add_folder(&a, "x");
    let other_x = server.add_folder(&twin, "x");
    let y = server.add_folder(&twin, "y");
    server.add_folder(&a, "unrequested");
    let dirs = vec![vec!["a".into(), "x".into()], vec!["a".into(), "y".into()]];
    let found = server
        .store
        .runtime
        .block_on(server.store.resolve_directories(&dirs, true))
        .unwrap();
    assert_eq!(found["a/x"], BTreeSet::from([x, other_x]));
    assert_eq!(found["a/y"], BTreeSet::from([y]));
    assert!(server.store.cached_folders("a/unrequested").is_none());
    assert_eq!(
        server.count("files"),
        6,
        "two root pages and four child pages"
    );
}

#[test]
fn batched_lookup_reads_again_for_a_later_batch_and_confirms_new_folders() {
    let server = Server::new();
    let a = server.add_folder(super::mock::ROOT, "a");
    let first = vec![vec!["a".into(), "x".into()], vec!["a".into(), "y".into()]];
    server
        .store
        .runtime
        .block_on(server.store.resolve_directories(&first, true))
        .unwrap();
    assert_eq!(server.count("create_folder"), 2);
    assert_eq!(
        server.count("files"),
        4,
        "root, parent, and two creation confirmations"
    );
    let z = server.add_folder(&a, "z");
    let before = server.count("files");
    let next = server
        .store
        .runtime
        .block_on(
            server
                .store
                .resolve_directories(&[vec!["a".into(), "z".into()]], true),
        )
        .unwrap();
    assert_eq!(next["a/z"], BTreeSet::from([z]));
    assert_eq!(
        server.count("files"),
        before + 1,
        "a later batch must list the parent again"
    );
    assert_eq!(server.count("create_folder"), 2);
}

#[test]
fn a_failed_batch_listing_does_not_create_folders_or_upload_files() {
    let server = Server::new();
    let objects = lookup_poc_fixture(&server, false);
    let parent_dirs = Location::of(&objects[0].0).unwrap().dirs[..5].to_vec();
    server
        .store
        .runtime
        .block_on(server.store.resolve(&parent_dirs, false, true))
        .unwrap();
    // The second parent-list page has an invalid file token.
    server.state.lock().unwrap().page_size = 1;
    let parents = server.folders_at(&parent_dirs.join("/"));
    let second = {
        let state = server.state.lock().unwrap();
        state
            .nodes
            .values()
            .filter(|node| node.parent == parents[0])
            .nth(1)
            .unwrap()
            .token
            .clone()
    };
    server
        .state
        .lock()
        .unwrap()
        .nodes
        .get_mut(&second)
        .unwrap()
        .token = "invalid/token".into();
    let mut reported = 0;
    assert!(server
        .store
        .put_many_with_progress(objects, &mut |_| reported += 1)
        .is_err());
    assert_eq!(server.count("create_folder"), 0);
    assert_eq!(server.count("upload_all"), 0);
    assert_eq!(reported, 0, "nothing settled, nothing reported");
}

fn expected(bytes: &[u8]) -> ExpectedObject {
    ExpectedObject {
        size: bytes.len() as u64,
        sha256: sha256_hex(bytes),
    }
}

fn verified_bytes(outcome: &StageOutcome) -> Vec<u8> {
    let StageOutcome::Verified(body) = outcome else {
        panic!("object was not staged: {outcome:?}")
    };
    body.read_bounded(1024, "object").unwrap()
}

#[test]
fn grouped_reads_resolve_forty_shards_with_forty_six_listings_and_no_lookup_per_download() {
    let server = Server::new();
    let objects = lookup_poc_fixture(&server, true);
    let requests: Vec<_> = objects
        .iter()
        .map(|(key, bytes)| (key.clone(), expected(bytes)))
        .collect();
    let sizes: Vec<u64> = objects
        .iter()
        .map(|(_, bytes)| bytes.len() as u64)
        .collect();
    let before = server.count("files");
    let mut reported = Vec::new();
    let outcomes = server
        .store
        .stage_many_with_progress(&requests, &mut |size| reported.push(size))
        .unwrap();
    for ((_, bytes), outcome) in objects.iter().zip(&outcomes) {
        assert_eq!(verified_bytes(outcome), bytes.as_ref());
    }
    let lists = server.count("files") - before;
    assert_eq!(
        lists, 46,
        "five ancestors and the parent once, then one listing per shard; a download adds none"
    );
    assert_eq!(server.count("download"), 40);
    assert_eq!(reported, sizes, "each request is reported once, in order");
    // Every token is now remembered: the same requests download without a
    // single listing.
    reported.clear();
    let outcomes = server
        .store
        .stage_many_with_progress(&requests, &mut |size| reported.push(size))
        .unwrap();
    assert!(outcomes
        .iter()
        .all(|outcome| matches!(outcome, StageOutcome::Verified(_))));
    assert_eq!(server.count("files") - before, lists);
    assert_eq!(server.count("download"), 80);
    assert_eq!(reported, sizes);
    println!("REQUEST_POC reads objects=40 operations=1 lists={lists} downloads=40 cached_lists=0");
}

#[test]
fn grouped_reads_union_duplicate_folders_settle_each_request_and_keep_order() {
    let server = Server::new();
    let first = b"first";
    let second = b"second";
    server
        .store
        .put_immutable(&digest_key(first), first)
        .unwrap();
    // Another computer created the same `_objects` folder at once and put
    // its object there; a twin of `first`'s shard holds a later copy with
    // other bytes. Reads take the union and the smallest token.
    let bundle_folder = server.folders_at(&format!(".mallard/v1/repositories/{BUNDLE}"))[0].clone();
    let twin = server.add_folder(&bundle_folder, "_objects");
    let second_shard = server.add_folder(&twin, &sha256_hex(second)[..2]);
    server.add_file(&second_shard, &sha256_hex(second), second);
    let first_twin_shard = server.add_folder(&twin, &sha256_hex(first)[..2]);
    server.add_file(&first_twin_shard, &sha256_hex(first), b"tampered");
    let shards = [&sha256_hex(first)[..2], &sha256_hex(second)[..2]];
    let absent_shard = (0..=255)
        .map(|byte| format!("{byte:02x}"))
        .find(|prefix| !shards.contains(&prefix.as_str()))
        .unwrap();
    let missing_in_shard = key(&format!("_objects/{}{}", shards[0], "0".repeat(62)));
    let missing_shard = key(&format!("_objects/{}{}", absent_shard, "1".repeat(62)));
    let missing_parent = ObjectKey::parse(format!(
        ".mallard/v1/repositories/{}/_objects/{}",
        "f".repeat(32),
        "2".repeat(64)
    ))
    .unwrap();
    let digest = |name: &ObjectKey| name.as_str().rsplit('/').next().unwrap().to_owned();
    let requests = vec![
        (digest_key(second), expected(second)),
        (digest_key(first), expected(first)),
        (
            missing_in_shard.clone(),
            ExpectedObject {
                size: 3,
                sha256: digest(&missing_in_shard),
            },
        ),
        (
            digest_key(first),
            ExpectedObject {
                size: first.len() as u64,
                sha256: "0".repeat(64),
            },
        ),
        (digest_key(second), expected(second)),
        (
            missing_shard.clone(),
            ExpectedObject {
                size: 4,
                sha256: digest(&missing_shard),
            },
        ),
        (
            missing_parent.clone(),
            ExpectedObject {
                size: 5,
                sha256: digest(&missing_parent),
            },
        ),
    ];
    // A store with nothing cached, over paginated listings.
    let other = server.second_store();
    server.state.lock().unwrap().page_size = 1;
    let mut reported = Vec::new();
    let outcomes = other
        .stage_many_with_progress(&requests, &mut |size| reported.push(size))
        .unwrap();
    assert_eq!(outcomes.len(), 7);
    assert_eq!(verified_bytes(&outcomes[0]), second);
    assert_eq!(verified_bytes(&outcomes[1]), first);
    assert!(matches!(outcomes[2], StageOutcome::Missing));
    assert!(matches!(outcomes[3], StageOutcome::Mismatch));
    assert_eq!(verified_bytes(&outcomes[4]), second);
    assert!(matches!(outcomes[5], StageOutcome::Missing));
    assert!(matches!(outcomes[6], StageOutcome::Missing));
    assert_eq!(
        server.count("download"),
        4,
        "a missing object is never downloaded; a digest check needs its bytes"
    );
    assert_eq!(reported, vec![6, 5, 3, 5, 6, 4, 5]);
    // The expected size bounds the download: a smaller expectation refuses
    // the bytes, a larger one reads them and fails verification.
    assert!(other
        .stage_many_with_progress(
            &[(
                digest_key(first),
                ExpectedObject {
                    size: 2,
                    sha256: sha256_hex(first)
                }
            )],
            &mut |_| {}
        )
        .is_err());
    assert!(matches!(
        other
            .stage_many_with_progress(
                &[(
                    digest_key(first),
                    ExpectedObject {
                        size: 6,
                        sha256: sha256_hex(first)
                    }
                )],
                &mut |_| {}
            )
            .unwrap()[0],
        StageOutcome::Mismatch
    ));
}

#[test]
fn a_replaced_file_reads_as_missing_without_a_lookup_until_the_next_call() {
    let server = Server::new();
    let payload = b"payload";
    server
        .store
        .put_immutable(&digest_key(payload), payload)
        .unwrap();
    let name = sha256_hex(payload);
    let token = {
        let state = server.state.lock().unwrap();
        state
            .nodes
            .values()
            .find(|node| node.kind == "file" && node.name == name)
            .unwrap()
            .token
            .clone()
    };
    let shard = server.folders_at(&format!(
        ".mallard/v1/repositories/{BUNDLE}/_objects/{}",
        &name[..2]
    ))[0]
        .clone();
    server.state.lock().unwrap().nodes.remove(&token);
    server.add_file(&shard, &name, payload);
    let requests = [(digest_key(payload), expected(payload))];
    let listings = server.count("files");
    let downloads = server.count("download");
    let mut reported = 0;
    let outcomes = server
        .store
        .stage_many_with_progress(&requests, &mut |_| reported += 1)
        .unwrap();
    assert!(matches!(outcomes[0], StageOutcome::Missing));
    assert_eq!(server.count("files"), listings, "nothing is resolved again");
    assert_eq!(server.count("download"), downloads + 1);
    assert_eq!(reported, 1);
    // The next call starts from what it lists: the replacement, verified.
    let outcomes = server.store.stage_many_verified(&requests).unwrap();
    assert_eq!(verified_bytes(&outcomes[0]), payload);
    assert_eq!(server.count("files"), listings + 1);
}

#[test]
fn a_later_page_error_fails_the_grouped_read_before_any_download() {
    let server = Server::new();
    let objects = lookup_poc_fixture(&server, true);
    let requests: Vec<_> = objects
        .iter()
        .map(|(key, bytes)| (key.clone(), expected(bytes)))
        .collect();
    server.state.lock().unwrap().page_size = 1;
    // A shard's second page holds an invalid token: the operation fails
    // with no download, and nothing is remembered from the broken folder.
    let shard = server.folders_at(&format!(
        ".mallard/v1/repositories/{BUNDLE}/_objects/{}",
        &sha256_hex(&objects[0].1)[..2]
    ))[0]
        .clone();
    let junk = server.add_file(&shard, "later-page", b"junk");
    server
        .state
        .lock()
        .unwrap()
        .nodes
        .get_mut(&junk)
        .unwrap()
        .token = "invalid/token".into();
    let mut reported = 0;
    assert!(server
        .store
        .stage_many_with_progress(&requests, &mut |_| reported += 1)
        .unwrap_err()
        .contains("file token"));
    assert_eq!(server.count("download"), 0);
    assert_eq!(reported, 0);
    server.state.lock().unwrap().nodes.remove(&junk);
    // The parent's second page is invalid: no shard is listed, no download.
    let parent_dirs = Location::of(&objects[0].0).unwrap().dirs[..5].to_vec();
    let parents = server.folders_at(&parent_dirs.join("/"));
    let second = {
        let state = server.state.lock().unwrap();
        state
            .nodes
            .values()
            .filter(|node| node.parent == parents[0])
            .nth(1)
            .unwrap()
            .token
            .clone()
    };
    server
        .state
        .lock()
        .unwrap()
        .nodes
        .get_mut(&second)
        .unwrap()
        .token = "invalid/token".into();
    *server.store.cache.lock().unwrap() = PathCache::default();
    assert!(server
        .store
        .stage_many_with_progress(&requests, &mut |_| reported += 1)
        .unwrap_err()
        .contains("file token"));
    assert_eq!(server.count("download"), 0);
    assert_eq!(reported, 0);
}

#[test]
fn grouped_reads_reject_an_entry_of_another_folder_and_find_nothing_under_a_foreign_root() {
    let server = Server::new();
    let objects = lookup_poc_fixture(&server, true);
    let requests: Vec<_> = objects
        .iter()
        .map(|(key, bytes)| (key.clone(), expected(bytes)))
        .collect();
    // A listing that names a child of some other folder is not a listing
    // of this one.
    server.state.lock().unwrap().response_once = Some((
        "files",
        serde_json::json!({"code":0,"data":{"files":[{"token":"node999999","name":".mallard","type":"folder","parent_token":"elsewhere","url":"https://example.invalid/not-followed"}],"has_more":false}}),
    ));
    assert_eq!(
        server
            .store
            .stage_many_with_progress(&requests, &mut |_| {})
            .unwrap_err(),
        "Feishu response failed validation (drive/v1/files: parent_token mismatch)."
    );
    assert_eq!(server.count("download"), 0);
    // Another storage folder holds none of these keys: every request is
    // missing, and no file of this folder is read on its behalf.
    let mut foreign = server.second_store();
    foreign.root = server.add_folder(super::mock::ROOT, "other");
    let outcomes = foreign
        .stage_many_with_progress(&requests, &mut |_| {})
        .unwrap();
    assert!(outcomes
        .iter()
        .all(|outcome| matches!(outcome, StageOutcome::Missing)));
    assert_eq!(server.count("download"), 0);
}

// Plant records without uploads, so scan request counts exclude fixture setup.
fn plant_listed_file(
    server: &Server,
    key: &ObjectKey,
    bytes: &[u8],
    dirs: &mut BTreeMap<String, String>,
) -> String {
    let location = Location::of(key).unwrap();
    let mut parent = super::mock::ROOT.to_string();
    let mut path = String::new();
    for part in location.dirs {
        path.push('/');
        path.push_str(&part);
        parent = dirs
            .entry(path.clone())
            .or_insert_with(|| server.add_folder(&parent, &part))
            .clone();
    }
    server.add_file(&parent, &location.name, bytes)
}

fn listed_record(
    revision: u64,
    parent: Option<&mallard_domain::RecordName>,
    bytes: &[u8],
) -> mallard_domain::RecordName {
    mallard_domain::RecordName::new(
        revision,
        mallard_domain::PublicationToken::new(format!("commit-{revision:024x}"), sha256_hex(bytes))
            .unwrap(),
        parent.map(|name| name.token().clone()),
        revision,
    )
    .unwrap()
}

#[test]
fn listed_records_read_without_cache_through_bare_and_configured_stores() {
    use crate::storage_core::publication::{list_stream, RecordStream};
    let server = Server::new();
    let stream = RecordStream::project(&mallard_domain::BundleId::parse(BUNDLE).unwrap()).unwrap();
    let name = listed_record(1, None, b"record");
    plant_listed_file(
        &server,
        &stream.key(&name).unwrap(),
        b"record",
        &mut BTreeMap::new(),
    );
    let configured = ConfiguredObjectStore {
        inner: Arc::new(super::mock::ListedReadStore(server.store.clone())),
        storage_id: mallard_domain::StorageId::parse("listed-test").unwrap(),
        publications: Arc::default(),
    };
    for store in [&server.store as &dyn ObjectStore, &configured] {
        let view = list_stream(store, &stream).unwrap();
        *server.store.cache.lock().unwrap() = PathCache::default();
        let before = server.count("files");
        let downloads = server.count("download");
        let selected = view.mainline().selected().unwrap();
        assert_eq!(
            view.read_record(store, &stream, selected).unwrap(),
            b"record"
        );
        assert_eq!(server.count("files"), before);
        assert_eq!(server.count("download"), downloads + 1);
    }
}

#[test]
fn listed_read_missing_never_resolves_a_replacement_and_rejects_foreign_folders() {
    let server = Server::new();
    let name = listed_record(1, None, b"record");
    let key = record_key(&name.file_name());
    let mut dirs = BTreeMap::new();
    let token = plant_listed_file(&server, &key, b"record", &mut dirs);
    let prefix =
        ObjectPrefix::parse(format!(".mallard/v1/repositories/_commits/{BUNDLE}")).unwrap();
    let objects = server.store.list_objects(&prefix, 100).unwrap();
    server.state.lock().unwrap().nodes.remove(&token);
    plant_listed_file(&server, &key, b"replacement", &mut dirs);
    *server.store.cache.lock().unwrap() = PathCache::default();
    let before = server.count("files");
    assert_eq!(
        server.store.get_listed_metadata_many(&objects, 64).unwrap(),
        vec![None]
    );
    assert_eq!(server.count("files"), before);
    let mut foreign = server.second_store();
    foreign.root = server.add_folder(super::mock::ROOT, "other");
    let requests = server.requests();
    assert!(foreign
        .get_listed_metadata_many(&objects, 64)
        .unwrap_err()
        .contains("another Feishu storage folder"));
    assert_eq!(server.requests(), requests);
    let fresh = server.store.list_objects(&prefix, 100).unwrap();
    assert_eq!(
        server.store.get_listed_metadata_many(&fresh, 64).unwrap()[0]
            .as_ref()
            .unwrap()
            .bytes,
        b"replacement"
    );
}

#[test]
fn listed_reads_preserve_order_bounds_and_record_digest_checks() {
    use crate::storage_core::publication::{list_stream, RecordStream};
    let server = Server::new();
    let stream = RecordStream::project(&mallard_domain::BundleId::parse(BUNDLE).unwrap()).unwrap();
    let first = listed_record(1, None, b"first");
    let second = listed_record(2, Some(&first), b"second");
    let mut dirs = BTreeMap::new();
    plant_listed_file(&server, &stream.key(&first).unwrap(), b"first", &mut dirs);
    let token = plant_listed_file(&server, &stream.key(&second).unwrap(), b"second", &mut dirs);
    let view = list_stream(&server.store, &stream).unwrap();
    let objects = [
        view.object(&stream, &second).unwrap(),
        view.object(&stream, &first).unwrap(),
        view.object(&stream, &second).unwrap(),
    ];
    *server.store.cache.lock().unwrap() = PathCache::default();
    let before = server.count("files");
    let bytes = server
        .store
        .get_listed_metadata_many(&objects, 64)
        .unwrap()
        .into_iter()
        .map(|stored| stored.unwrap().bytes)
        .collect::<Vec<_>>();
    assert_eq!(
        bytes,
        [b"second".to_vec(), b"first".to_vec(), b"second".to_vec()]
    );
    assert!(server.store.get_listed_metadata_many(&objects, 5).is_err());
    server
        .state
        .lock()
        .unwrap()
        .nodes
        .get_mut(&token)
        .unwrap()
        .bytes = b"broken".to_vec();
    assert!(view
        .read_record(&server.store, &stream, &second)
        .unwrap_err()
        .contains("digest"));
    assert_eq!(server.count("files"), before);
}

#[test]
fn large_publication_scans_walk_once_and_each_scan_is_fresh() {
    use crate::storage_core::publication::{list_stream, RecordStream};
    let server = Server::new();
    let stream = RecordStream::project(&mallard_domain::BundleId::parse(BUNDLE).unwrap()).unwrap();
    let mut dirs = BTreeMap::new();
    let mut last = None;
    for revision in 1..=10_001 {
        let name = listed_record(revision, last.as_ref(), b"record");
        plant_listed_file(&server, &stream.key(&name).unwrap(), b"record", &mut dirs);
        last = Some(name);
    }
    // Five parent folders, the stream folder, and 51 shard pages.
    for _ in 0..2 {
        let before = server.count("files");
        let view = list_stream(&server.store, &stream).unwrap();
        assert_eq!(view.mainline().selected(), last.as_ref());
        assert_eq!(server.count("files") - before, 57);
        *server.store.cache.lock().unwrap() = PathCache::default();
        let before = server.count("files");
        assert_eq!(
            view.read_record(&server.store, &stream, view.mainline().selected().unwrap())
                .unwrap(),
            b"record"
        );
        assert_eq!(server.count("files"), before);
    }
    let next = listed_record(10_002, last.as_ref(), b"new");
    plant_listed_file(&server, &stream.key(&next).unwrap(), b"new", &mut dirs);
    assert_eq!(
        list_stream(&server.store, &stream)
            .unwrap()
            .mainline()
            .selected(),
        Some(&next)
    );
}

#[test]
fn complete_scan_unions_duplicate_folders_and_rejects_invalid_later_pages() {
    let server = Server::new();
    server.state.lock().unwrap().page_size = 1;
    let name = listed_record(1, None, b"record");
    let key = record_key(&name.file_name());
    let first = plant_listed_file(&server, &key, b"record", &mut BTreeMap::new());
    // A duplicate root, created after a completed scan, must also be walked.
    let prefix =
        ObjectPrefix::parse(format!(".mallard/v1/repositories/_commits/{BUNDLE}")).unwrap();
    server.store.list_objects(&prefix, 10).unwrap();
    let mut dirs = BTreeMap::new();
    plant_listed_file(&server, &key, b"duplicate", &mut dirs);
    let next = listed_record(2, Some(&name), b"next");
    let later = plant_listed_file(&server, &record_key(&next.file_name()), b"next", &mut dirs);
    let listed = server.store.list_objects(&prefix, 10).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].feishu.as_ref().unwrap().token, first);
    assert!(server.store.list_objects(&prefix, 1).is_err());
    // This is the second page of the duplicate shard. No partial scan escapes.
    server
        .state
        .lock()
        .unwrap()
        .nodes
        .get_mut(&later)
        .unwrap()
        .token = "invalid/token".into();
    let downloads = server.count("download");
    assert!(server
        .store
        .list_objects(&prefix, 10)
        .unwrap_err()
        .contains("file token"));
    assert_eq!(server.count("download"), downloads);
    assert!(server.store.list(&prefix, None, MAX_LIST_PAGE + 1).is_err());
}
