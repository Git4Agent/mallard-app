#[test]
fn claude_delivered_files_show_with_replies_locally_and_in_cloud_views() {
    use mallard_infrastructure::bundle_engine::{BundleBytes, BundleEngine};
    use mallard_infrastructure::managed_cloud::views::build_view;
    use mallard_infrastructure::storage_core::LocalObjectStore;

    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("delivery-store").unwrap();
    let store_root = temp.path().join("store");
    add_local_storage(&repo, &storage, &store_root);
    let source = register(&repo);
    let source_root = temp.path().join("source");
    let source_home = temp.path().join("source-claude");
    bind_and_link_claude_replica(&repo, &source, &storage, &source_root, &source_home);
    let parent = "22222222-2222-4222-8222-222222222222";
    let film = source_root.join("The_Iliad_Complete.mp4");
    let subtitles = source_root.join("The_Iliad_Complete.srt");
    let outside = temp.path().join("elsewhere/clip.mp4");
    let mut bytes = claude_test_transcript(
        parent,
        &source_root,
        &[("u1", None, "user", "Show me the complete film")],
    );
    for record in [
        serde_json::json!({"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":parent,"cwd":source_root,
        "message":{"id":"message-1","role":"assistant","stop_reason":"tool_use","content":[
            {"type":"tool_use","id":"send-1","name":"SendUserFile","input":{"files":[film, subtitles, outside],"caption":"The Iliad — Complete","display":"render","status":"normal"}}
        ]}}),
        serde_json::json!({"type":"user","uuid":"r1","parentUuid":"a1","sessionId":parent,"cwd":source_root,
        "toolUseResult":{"attachments":[{"path":film,"size":281008852,"isImage":false,"media_type":"video/mp4"}]},
        "message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"send-1","content":"2 files delivered to user."}
        ]}}),
        serde_json::json!({"type":"assistant","uuid":"a2","parentUuid":"r1","sessionId":parent,"cwd":source_root,
            "message":{"id":"message-2","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"I've attached the complete film above."}]}}),
    ] {
        serde_json::to_writer(&mut bytes, &record).unwrap();
        bytes.push(b'\n');
    }
    std::fs::write(
        source_home
            .join("projects/source")
            .join(format!("{parent}.jsonl")),
        bytes,
    )
    .unwrap();
    let service = ProjectSyncService::new(&repo);
    let provider = ProviderKey::parse("claude").unwrap();
    let review = service
        .prepare_push_review(&source.local_project_id, &storage)
        .unwrap();
    let pushed = service
        .push_reviewed(
            &source.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(pushed.success, "{} {:?}", pushed.message, pushed.results);

    // The desktop's default Replies query shows the delivery beside the reply,
    // naming the files by their project paths: nothing absolute crosses.
    let bindings = repo.load_conversation_bindings().unwrap();
    let binding = bindings
        .bindings
        .iter()
        .find(|row| {
            row.local_project_id == source.local_project_id
                && row.provider_thread_id.as_deref() == Some(parent)
        })
        .unwrap();
    let resource_id = conversation_resource_id(&provider, &binding.thread_id).unwrap();
    let detail = service
        .get_project_session_detail(&source.local_project_id, &resource_id)
        .unwrap();
    let page = service
        .get_project_session_timeline_page(
            &source.local_project_id,
            &resource_id,
            ProjectSessionTimelineRequest {
                snapshot_id: detail.snapshot_id,
                cursor: None,
                limit: Some(50),
                included_event_kinds: BTreeSet::from([SessionTimelineEventKind::Response]),
                included_tool_categories: BTreeSet::new(),
                turn_id: None,
            },
        )
        .unwrap();
    assert_eq!(page.events.len(), 2);
    let delivery = page.events[0].tool.as_ref().unwrap();
    assert_eq!(delivery.native_name, "SendUserFile");
    assert_eq!(
        delivery.affected_paths,
        vec!["The_Iliad_Complete.mp4", "The_Iliad_Complete.srt"]
    );
    assert_eq!(delivery.summary, "The_Iliad_Complete.mp4");
    assert_eq!(page.events[1].kind, SessionTimelineEventKind::Response);

    // The website's view, built from the published wire object, indexes the
    // same project paths from the portable root and carries no local root.
    let engine = BundleEngine::open(
        LocalObjectStore::open(&store_root).unwrap(),
        storage.clone(),
    )
    .unwrap();
    let snapshot = engine.inspect(&source.bundle_id).unwrap();
    let scope = snapshot.manifest.provider_scope(&provider).unwrap();
    let thread = scope.conversations.values().next().unwrap();
    let transcript = thread
        .files
        .iter()
        .find(|file| file.role == ConversationFileRole::Transcript)
        .unwrap();
    let wire = engine
        .byte_source(&snapshot, None)
        .read_bounded(&transcript.logical_path, 1024 * 1024, "delivery fixture")
        .unwrap();
    let view = build_view(
        provider_registry(),
        &provider,
        thread.resource_id.as_str(),
        parent,
        None,
        &wire,
    )
    .unwrap();
    let preview: serde_json::Value = serde_json::from_slice(&view.bytes).unwrap();
    let delivery = preview["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["tool"]["native_name"] == "SendUserFile")
        .unwrap();
    assert_eq!(
        delivery["tool"]["affected_paths"],
        serde_json::json!(["The_Iliad_Complete.mp4", "The_Iliad_Complete.srt"])
    );
    assert_eq!(delivery["tool"]["summary"], "The_Iliad_Complete.mp4");
    let arguments = delivery["tool"]["argument_preview"].as_str().unwrap();
    assert!(arguments.contains("/__mallard_root__/"), "{arguments}");
    let local_root = source_root.to_string_lossy();
    assert!(!String::from_utf8_lossy(&view.bytes).contains(local_root.as_ref()));
}
