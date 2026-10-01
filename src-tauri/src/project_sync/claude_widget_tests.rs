#[test]
fn claude_svg_widget_survives_push_pull_and_cloud_preview() {
    use mallard_infrastructure::bundle_engine::{BundleBytes, BundleEngine};
    use mallard_infrastructure::managed_cloud::views::build_view;
    use mallard_infrastructure::storage_core::LocalObjectStore;

    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("widget-store").unwrap();
    let store_root = temp.path().join("store");
    add_local_storage(&repo, &storage, &store_root);
    let source = register(&repo);
    let source_root = temp.path().join("source");
    let source_home = temp.path().join("source-claude");
    bind_and_link_claude_replica(&repo, &source, &storage, &source_root, &source_home);
    let parent = "11111111-1111-4111-8111-111111111111";
    let svg = include_str!("../../../poc/claude-widgets/web/diagram.svg");
    let mermaid = "Here is the diagram.\n\n```mermaid\nflowchart TD\nA[Portable 🦆] --> B[Local and Cloud]\n```";
    let mut bytes = claude_test_transcript(
        parent,
        &source_root,
        &[("u1", None, "user", "Show the diagram")],
    );
    for record in [
        serde_json::json!({"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":parent,"cwd":source_root,
        "message":{"id":"message-1","role":"assistant","stop_reason":"tool_use","content":[
            {"type":"tool_use","id":"read-1","name":"Read","input":{"file_path":"README.md"}},
            {"type":"tool_use","id":"widget-1","name":"mcp__visualize__show_widget","input":{"title":"Portable diagram","widget_code":svg}}
        ]}}),
        serde_json::json!({"type":"user","uuid":"r1","parentUuid":"a1","sessionId":parent,"cwd":source_root,
        "message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"read-1","content":"Fixture source"},
            {"type":"tool_result","tool_use_id":"widget-1","content":"Widget displayed"}
        ]}}),
        serde_json::json!({"type":"assistant","uuid":"a2","parentUuid":"r1","sessionId":parent,"cwd":source_root,
            "message":{"id":"message-2","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":mermaid}]}}),
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

    // Build the website view from the actual verified, published wire object.
    let provider = ProviderKey::parse("claude").unwrap();
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
        .read_bounded(&transcript.logical_path, 1024 * 1024, "widget fixture")
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
    let widget = preview["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["tool"]["native_name"] == "mcp__visualize__show_widget")
        .unwrap();
    assert_eq!(widget["tool"]["argument_truncated"], false);
    let arguments: serde_json::Value =
        serde_json::from_str(widget["tool"]["argument_preview"].as_str().unwrap()).unwrap();
    assert_eq!(arguments["widget_code"], svg);
    let response = preview["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["kind"] == "response")
        .unwrap();
    assert_eq!(response["text"], mermaid);

    let target = register_local_project_with_repository(
        &repo,
        RegisterLocalProjectRequest {
            display_name: "Widget destination".into(),
            repository_fingerprint: source.repository_fingerprint.clone(),
            bundle_id: Some(source.bundle_id.clone()),
        },
    )
    .unwrap();
    let target_root = temp.path().join("target");
    let target_home = temp.path().join("target-claude");
    bind_and_link_claude_replica(&repo, &target, &storage, &target_root, &target_home);
    let bindings = repo.load_bindings().unwrap();
    let plan = service
        .plan_bundle_restore(
            &storage,
            &source.bundle_id,
            bindings.active_for(&target.local_project_id).unwrap(),
        )
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
    assert!(
        applied.success,
        "{} {:?}",
        applied.message, applied.failed_actions
    );
    let conversation_bindings = repo.load_conversation_bindings().unwrap();
    let installed = conversation_bindings
        .bindings
        .iter()
        .find(|row| row.local_project_id == target.local_project_id)
        .unwrap();
    let dest = installed.provider_thread_id.as_ref().unwrap();
    assert_ne!(dest, parent);
    let mut files = Vec::new();
    jsonl_files_under(&target_home, &mut files);
    let path = files
        .iter()
        .find(|path| path.file_name().unwrap() == format!("{dest}.jsonl").as_str())
        .unwrap();
    let restored = std::fs::read_to_string(path).unwrap();
    let record = restored
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|row| row["uuid"] == "a1")
        .unwrap();
    assert_eq!(record["message"]["content"][1]["input"]["widget_code"], svg);
    let response = restored
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|row| row["uuid"] == "a2")
        .unwrap();
    assert_eq!(response["message"]["content"][0]["text"], mermaid);

    // The receiving desktop's default Replies query shows the widget, but not Read.
    let resource_id = conversation_resource_id(&provider, &installed.thread_id).unwrap();
    let detail = service
        .get_project_session_detail(&target.local_project_id, &resource_id)
        .unwrap();
    let request = ProjectSessionTimelineRequest {
        snapshot_id: detail.snapshot_id,
        cursor: None,
        limit: Some(50),
        included_event_kinds: BTreeSet::from([SessionTimelineEventKind::Response]),
        included_tool_categories: BTreeSet::new(),
        turn_id: None,
    };
    let page = service
        .get_project_session_timeline_page(&target.local_project_id, &resource_id, request.clone())
        .unwrap();
    assert_eq!(page.events.len(), 2);
    assert_eq!(
        page.events
            .iter()
            .find(|event| event.kind == SessionTimelineEventKind::Response)
            .unwrap()
            .text
            .as_deref(),
        Some(mermaid)
    );
    let tool = page
        .events
        .iter()
        .find_map(|event| event.tool.as_ref())
        .unwrap();
    assert_eq!(tool.native_name, "mcp__visualize__show_widget");
    assert!(!tool.argument_truncated);
    let arguments: serde_json::Value =
        serde_json::from_str(tool.argument_preview.as_deref().unwrap()).unwrap();
    assert_eq!(arguments["widget_code"], svg);

    let first = service
        .get_project_session_timeline_page(
            &target.local_project_id,
            &resource_id,
            ProjectSessionTimelineRequest {
                limit: Some(1),
                ..request.clone()
            },
        )
        .unwrap();
    assert_eq!(first.events.len(), 1);
    assert!(first.events[0].tool.is_some());
    assert!(first.next_cursor.is_some());
    let second = service
        .get_project_session_timeline_page(
            &target.local_project_id,
            &resource_id,
            ProjectSessionTimelineRequest {
                limit: Some(1),
                cursor: first.next_cursor,
                ..request.clone()
            },
        )
        .unwrap();
    assert_eq!(second.events.len(), 1);
    assert_eq!(second.events[0].kind, SessionTimelineEventKind::Response);
    assert!(second.next_cursor.is_none());
    let other_turn = service
        .get_project_session_timeline_page(
            &target.local_project_id,
            &resource_id,
            ProjectSessionTimelineRequest {
                turn_id: Some("absent-turn".into()),
                ..request.clone()
            },
        )
        .unwrap();
    assert!(
        other_turn.events.is_empty(),
        "widget output must still respect the turn filter"
    );
    let reads = service
        .get_project_session_timeline_page(
            &target.local_project_id,
            &resource_id,
            ProjectSessionTimelineRequest {
                included_event_kinds: BTreeSet::from([SessionTimelineEventKind::ToolCall]),
                included_tool_categories: BTreeSet::from([SessionToolCategory::Read]),
                ..request
            },
        )
        .unwrap();
    assert_eq!(reads.events.len(), 1);
    assert_eq!(reads.events[0].tool.as_ref().unwrap().native_name, "Read");
}

#[test]
#[ignore = "Prepare an isolated full desktop and Cloud session fixture"]
fn mermaid_native_ui_fixture() {
    use mallard_infrastructure::bundle_engine::{BundleBytes, BundleEngine};
    use mallard_infrastructure::managed_cloud::views::build_view;
    use mallard_infrastructure::storage_core::LocalObjectStore;

    let root = PathBuf::from(std::env::var("MALLARD_MERMAID_QA_ROOT").unwrap());
    let root = std::fs::canonicalize(root).unwrap();
    assert!(root.starts_with("/private/tmp"));
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("mallard-mermaid-qa-"));
    assert!(
        !root.join("home/.mallard").exists(),
        "Use a fresh fixture root"
    );
    let repo = LocalStateRepository::from_home_dir(root.join("home")).unwrap();
    let storage = StorageId::parse("mermaid-qa").unwrap();
    add_local_storage(&repo, &storage, &root.join("store"));
    let project = register_local_project_with_repository(
        &repo,
        RegisterLocalProjectRequest {
            display_name: "Session diagrams QA".into(),
            repository_fingerprint: None,
            bundle_id: Some(BundleId::parse("0123456789abcdef0123456789abcdef").unwrap()),
        },
    )
    .unwrap();
    let project_root = root.join("project");
    let claude_home = root.join("claude");
    bind_and_link_claude_replica(&repo, &project, &storage, &project_root, &claude_home);
    let parent = "11111111-1111-4111-8111-111111111111";
    let diagram = include_str!("../../../poc/mermaid-flowcharts/supplied-flowchart.mmd");
    let svg = include_str!("../../../poc/mermaid-flowcharts/child-transcripts.svg");
    let text =
        format!("The saved flowchart follows the session theme.\n\n```mermaid\n{diagram}```");
    let mut bytes = claude_test_transcript(
        parent,
        &project_root,
        &[(
            "u1",
            None,
            "user",
            "Flowcharts and child transcript widgets",
        )],
    );
    for record in [
        serde_json::json!({"type":"assistant","uuid":"a1","parentUuid":"u1","sessionId":parent,"cwd":project_root,
        "message":{"id":"message-1","role":"assistant","stop_reason":"tool_use","content":[
            {"type":"tool_use","id":"widget-1","name":"mcp__visualize__show_widget","input":{"title":"child_transcripts_held_by_each_side","widget_code":svg}}
        ]}}),
        serde_json::json!({"type":"user","uuid":"r1","parentUuid":"a1","sessionId":parent,"cwd":project_root,
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"widget-1","content":"Widget displayed"}]}}),
        serde_json::json!({"type":"assistant","uuid":"a2","parentUuid":"r1","sessionId":parent,"cwd":project_root,
            "message":{"id":"message-2","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":text}]}}),
    ] {
        serde_json::to_writer(&mut bytes, &record).unwrap();
        bytes.push(b'\n');
    }
    std::fs::write(
        claude_home
            .join("projects/source")
            .join(format!("{parent}.jsonl")),
        &bytes,
    )
    .unwrap();
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&project.local_project_id, &storage)
        .unwrap();
    let pushed = service
        .push_reviewed(
            &project.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(pushed.success, "{} {:?}", pushed.message, pushed.results);
    let engine =
        BundleEngine::open(LocalObjectStore::open(root.join("store")).unwrap(), storage).unwrap();
    let snapshot = engine.inspect(&project.bundle_id).unwrap();
    let provider = ProviderKey::parse("claude").unwrap();
    let thread = snapshot
        .manifest
        .provider_scope(&provider)
        .unwrap()
        .conversations
        .values()
        .next()
        .unwrap();
    let file = thread
        .files
        .iter()
        .find(|file| file.role == ConversationFileRole::Transcript)
        .unwrap();
    let wire = engine
        .byte_source(&snapshot, None)
        .read_bounded(&file.logical_path, 1024 * 1024, "diagram fixture")
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
    std::fs::write(root.join("cloud-view.json"), view.bytes).unwrap();
    std::fs::write(
        root.join("cloud-manifest.json"),
        serde_json::to_vec(&snapshot.manifest).unwrap(),
    )
    .unwrap();
    let bindings = repo.load_conversation_bindings().unwrap();
    let binding = bindings
        .bindings
        .iter()
        .find(|row| {
            row.local_project_id == project.local_project_id
                && row.provider_thread_id.as_deref() == Some(parent)
        })
        .unwrap();
    let local_resource = conversation_resource_id(&provider, &binding.thread_id).unwrap();
    let detail = service
        .get_project_session_detail(&project.local_project_id, &local_resource)
        .unwrap();
    assert!(detail.snapshot_id.as_str().len() > 0);
    std::fs::write(
        root.join("fixture.json"),
        serde_json::to_vec(&serde_json::json!({
            "project_id": project.local_project_id, "resource_id": thread.resource_id,
            "commit_id": snapshot.manifest.commit_id,
        }))
        .unwrap(),
    )
    .unwrap();
}
