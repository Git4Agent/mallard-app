//! Native Claude cwd changes, reviewed media transport, and derived cloud views.
//! All model responses and the video are synthetic; every home is disposable.
use super::*;
use mallard_domain::portable::contracts;
use mallard_domain::{ConversationFileRole, LogicalPath, RecipeEntry};
use mallard_infrastructure::managed_cloud::views::build_view;
use mallard_infrastructure::providers::AgentProviderRegistry;

const VIDEO: &str = "Part5_The_Fall_of_Patroclus_zh.mp4";
const VIDEO_BYTES: &[u8] = include_bytes!("../../../tests/fixtures/claude-media/clip.mp4");
const LARGE_FILE: &str = "transfer-200mb.bin";
// Decimal MB, exactly as reported by the test. Generated only under its temp root.
const LARGE_FILE_BYTES: u64 = 200_000_000;
const IO_BUFFER_BYTES: usize = 64 * 1024;

const SESSION: &str = "018f3f2e-7b91-7ad0-8000-000000000401";
const PROMPT: &str = "MALLARD_CLAUDE_USER_MEDIA";
const RESPONSE: &str = "MALLARD_CLAUDE_MEDIA_READY\n[Part5_The_Fall_of_Patroclus_zh.mp4](../Part5_The_Fall_of_Patroclus_zh.mp4)";
const RESUME_PROMPT: &str = "MALLARD_CLAUDE_USER_MEDIA_RESUME";
const RESUME_RESPONSE: &str = "MALLARD_CLAUDE_MEDIA_RESUMED";
const CALL: &str = "mallard_claude_media_cd";

fn write_large_file(path: &Path) -> String {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .expect("create private large-file fixture");
    let mut digest = Sha256::new();
    let mut buffer = [0xa5_u8; IO_BUFFER_BYTES];
    let mut written = 0_u64;
    // Distinct block offsets prevent identical chunks from hiding a reordering bug.
    while written < LARGE_FILE_BYTES {
        buffer[..8].copy_from_slice(&written.to_le_bytes());
        let count = (LARGE_FILE_BYTES - written).min(buffer.len() as u64) as usize;
        file.write_all(&buffer[..count])
            .expect("write large-file fixture block");
        digest.update(&buffer[..count]);
        written += count as u64;
    }
    file.flush().expect("flush large-file fixture");
    assert_eq!(file.metadata().unwrap().len(), LARGE_FILE_BYTES);
    format!("{:x}", digest.finalize())
}

fn assert_file_digest(path: &Path, size: u64, expected_digest: &str) {
    let metadata = std::fs::symlink_metadata(path).expect("inspect transferred file");
    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    assert_eq!(metadata.len(), size, "transferred file size changed");
    let mut file = std::fs::File::open(path).unwrap().take(size + 1);
    let mut buffer = [0_u8; IO_BUFFER_BYTES];
    let mut digest = Sha256::new();
    let mut read = 0_u64;
    loop {
        let count = file.read(&mut buffer).expect("read transferred file block");
        if count == 0 {
            break;
        }
        read += count as u64;
        assert!(read <= size, "transferred file grew during verification");
        digest.update(&buffer[..count]);
    }
    assert_eq!(read, size, "transferred file was truncated");
    assert_eq!(format!("{:x}", digest.finalize()), expected_digest);
}

fn scenarios(source: &Path, destination: &Path) -> AimockScenarioDocument {
    let initial = AimockScenario {
        id: "media_cd".into(),
        prompt_marker: PROMPT.into(),
        expected_history: Vec::new(),
        require_tool_result: false,
        expected_tool_call_id: None,
        expected_tool_result_marker: None,
        response: AimockResponse::ChangeDirectory {
            call_id: CALL.into(),
            directory: source.join("part5").to_string_lossy().into_owned(),
            restored_directory: destination.join("part5").to_string_lossy().into_owned(),
        },
    };
    let mut final_reply = text_scenario("media_link", PROMPT, Vec::new(), RESPONSE);
    final_reply.require_tool_result = true;
    final_reply.expected_tool_call_id = Some(CALL.into());
    final_reply.expected_tool_result_marker = Some("MALLARD_CLAUDE_CHANGED_DIRECTORY".into());
    AimockScenarioDocument {
        version: 1,
        model: MODEL.into(),
        expected_request_count: 3,
        expected_probe_request_count: 2,
        scenarios: vec![
            initial,
            final_reply,
            text_scenario(
                "media_resume",
                RESUME_PROMPT,
                marker_turn(PROMPT, RESPONSE),
                RESUME_RESPONSE,
            ),
        ],
    }
}

fn assert_media_record(bytes: &[u8], expected_cwd: &str) {
    let rows = bytes
        .split(|byte| *byte == b'\n')
        .filter(|row| !row.is_empty())
        .map(|row| serde_json::from_slice::<Value>(row).expect("media transcript JSON"))
        .filter(|row| {
            row["type"] == "assistant"
                && row["message"]["content"]
                    .as_array()
                    .is_some_and(|blocks| blocks.iter().any(|block| block["text"] == RESPONSE))
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "media response must survive exactly once");
    assert!(
        rows[0]["cwd"].as_str() == Some(expected_cwd),
        "media response cwd was not preserved"
    );
}

fn assert_detail(
    service: &ProjectSyncService<'_>,
    project: &LocalProjectId,
    resource: &ResourceId,
) {
    let header = service
        .get_project_session_detail(project, resource)
        .unwrap();
    let page = service
        .get_project_session_timeline_page(
            project,
            resource,
            ProjectSessionTimelineRequest {
                snapshot_id: header.snapshot_id,
                cursor: None,
                limit: Some(50),
                included_event_kinds: BTreeSet::from([SessionTimelineEventKind::Response]),
                included_tool_categories: BTreeSet::new(),
                turn_id: None,
            },
        )
        .unwrap();
    assert!(page.next_cursor.is_none());
    let responses = page
        .events
        .iter()
        .filter(|event| event.text.as_deref() == Some(RESPONSE))
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].working_directory.as_deref(), Some("part5"));
}

fn push_media(service: &ProjectSyncService<'_>, project: &LocalProjectId, storage: &StorageId) {
    let files = service.inspect_project_files(project, storage).unwrap();
    let review = service.prepare_push_review(project, storage).unwrap();
    let mut approval = PushApproval::recommended(&review, storage);
    let selected = files
        .entries
        .iter()
        .filter(|entry| matches!(entry.relative_path.as_str(), VIDEO | LARGE_FILE | "part5"))
        .collect::<Vec<_>>();
    assert_eq!(
        selected.len(),
        3,
        "review must include the video, 200 MB file, and message directory"
    );
    for entry in selected {
        assert!(entry.local_present && entry.blocked_reason.is_none());
        assert!(
            entry.warning_code.is_none(),
            "synthetic media unexpectedly requires acknowledgement"
        );
        let resource_id = entry.descriptor.resource_id.clone();
        approval.recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id,
                apply_policy: contracts::explicit_review(),
                required: entry.relative_path == "part5",
            },
        );
    }
    approval.project_content_review_token = files.review_token;
    approval.project_files_shown = true;
    let result = service.push_reviewed(project, storage, approval).unwrap();
    assert!(result.success, "reviewed media Push failed");
    assert_eq!(result.revision, Some(2));
}

pub(super) fn run() {
    println!("live Claude phase: nested cwd and reviewed video transport");
    let root = tempfile::tempdir().unwrap();
    let process = root.path().join("process");
    let projects = [
        root.path().join("machine-a/Homer"),
        root.path().join("machine-b/Homer"),
    ];
    let homes = [root.path().join("claude-a"), root.path().join("claude-b")];
    let machines = [root.path().join("state-a"), root.path().join("state-b")];
    let storage_root = root.path().join("storage");
    let control = root.path().join("control");
    for path in [
        &process,
        &projects[0],
        &projects[1],
        &homes[0],
        &homes[1],
        &machines[0],
        &machines[1],
        &storage_root,
        &control,
    ] {
        secure_directory(path);
    }
    let projects = projects.map(|path| canonicalize(&path).unwrap());
    secure_directory(&projects[0].join("part5"));
    write_private_new(&projects[0].join(VIDEO), VIDEO_BYTES);
    println!("Claude media: generate and transfer exactly {LARGE_FILE_BYTES} bytes");
    let large_digest = write_large_file(&projects[0].join(LARGE_FILE));
    let _home = EnvironmentGuard::set("HOME", &process);
    let _userprofile = EnvironmentGuard::set("USERPROFILE", &process);
    assert_supported_claude_version(&process, EXPECTED_AIMOCK_CLAUDE_VERSION);
    let repositories = machines
        .each_ref()
        .map(|path| LocalStateRepository::from_home_dir(path).unwrap());
    let services = repositories.each_ref().map(ProjectSyncService::new);
    let storage = local_storage(&storage_root);
    let a = finish_setup(
        &repositories[0],
        &services[0],
        &projects[0],
        &homes[0],
        storage.clone(),
        DraftRepositoryChoice::New,
    );
    let bundle = a.project.bundle_id.clone();
    let (aimock, provider) =
        AimockProcess::start(&control, &process, &scenarios(&projects[0], &projects[1])).unwrap();
    let mut processes = 0;
    run_claude_with_tool(
        &mut processes,
        &provider,
        &homes[0],
        &process,
        &projects[0],
        SESSION,
        true,
        PROMPT,
        RESPONSE,
        "Bash",
    );
    let source = transcript_bytes(&homes[0], &projects[0], SESSION);
    assert_media_record(&source, projects[0].join("part5").to_str().unwrap());
    push_media(&services[0], &a.project.local_project_id, &storage.id);
    assert_eq!(transcript_bytes(&homes[0], &projects[0], SESSION), source);
    let a_thread = logical_thread_id(&repositories[0], &a.project.local_project_id, SESSION);
    let cloud = cloud_thread_id_of(&repositories[0], &a.project.local_project_id, &a_thread);
    assert_detail(
        &services[0],
        &a.project.local_project_id,
        &mallard_domain::conversation_resource_id(&claude_key(), &a_thread).unwrap(),
    );

    let snapshot = services[0].inspect_bundle(&storage.id, &bundle).unwrap();
    let (_, _, conversation) = snapshot
        .manifest
        .all_conversations()
        .find(|(_, thread, _)| *thread == &cloud)
        .unwrap();
    let transcript = conversation
        .files
        .iter()
        .find(|file| matches!(file.role, ConversationFileRole::Transcript))
        .unwrap();
    let stored = snapshot
        .manifest
        .find_file(&transcript.logical_path)
        .unwrap()
        .1;
    assert!(stored.chunks.is_empty());
    let objects = storage_root
        .join(".mallard/v1/repositories")
        .join(bundle.as_str())
        .join("_objects");
    let wire = read_bounded(
        &objects.join(&stored.sha256),
        MAX_CONVERSATION_FILE_BYTES,
        "stored media transcript",
    );
    assert_eq!(mallard_domain::sha256_hex(&wire), stored.sha256);
    assert_media_record(&wire, "/__mallard_root__/b;/part5");
    let media = snapshot
        .manifest
        .find_file(&LogicalPath::parse(format!("project/{VIDEO}")).unwrap())
        .unwrap()
        .1;
    assert_eq!(media.sha256, mallard_domain::sha256_hex(VIDEO_BYTES));
    let large = snapshot
        .manifest
        .find_file(&LogicalPath::parse(format!("project/{LARGE_FILE}")).unwrap())
        .expect("200 MB file must be published")
        .1;
    assert_eq!(large.size, LARGE_FILE_BYTES);
    assert_eq!(large.sha256, large_digest);
    assert!(
        large.object_key.is_none(),
        "large file must use chunk objects"
    );
    assert_eq!(large.chunks.len(), 6);
    assert_eq!(
        large
            .chunks
            .iter()
            .map(|chunk| &chunk.sha256)
            .collect::<BTreeSet<_>>()
            .len(),
        6,
        "fixture chunks must be distinct"
    );
    let mut remaining = LARGE_FILE_BYTES;
    for chunk in &large.chunks {
        assert_eq!(
            chunk.size,
            remaining.min(mallard_domain::PAYLOAD_CHUNK_BYTES)
        );
        assert_file_digest(&objects.join(&chunk.sha256), chunk.size, &chunk.sha256);
        remaining -= chunk.size;
    }
    assert_eq!(remaining, 0);
    assert!(
        !objects.join(&large.sha256).exists(),
        "large file must not also be stored whole"
    );
    let view = build_view(
        AgentProviderRegistry::production(),
        &claude_key(),
        "claude:conversation:media",
        SESSION,
        None,
        &wire,
    )
    .unwrap();
    let view_json: Value = serde_json::from_slice(&view.bytes).unwrap();
    assert_eq!(view_json["schema_version"], 1);
    assert_eq!(view.source_sha256, stored.sha256);
    let responses = view_json["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["text"] == RESPONSE)
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["working_directory"], "part5");

    let b = finish_setup(
        &repositories[1],
        &services[1],
        &projects[1],
        &homes[1],
        storage.clone(),
        DraftRepositoryChoice::Existing {
            storage_id: storage.id.clone(),
            bundle_id: bundle.clone(),
            display_name: a.project.display_name.clone(),
            repository_fingerprint: a.project.repository_fingerprint.clone(),
            mismatch_acknowledged: true,
        },
    );
    assert!(
        !projects[1].join(VIDEO).exists(),
        "video must arrive through Pull"
    );
    assert!(
        !projects[1].join(LARGE_FILE).exists(),
        "200 MB file must arrive through Pull"
    );
    let (plan, _) = pull_and_apply(
        &services[1],
        &storage.id,
        &bundle,
        b.binding.as_ref().unwrap(),
    );
    assert!(plan.actions.iter().any(|action| matches!(&action.kind, RestoreActionKind::WriteProjectFile { logical_path, .. } if logical_path.as_str() == format!("project/{VIDEO}"))));
    assert!(plan.actions.iter().any(|action| matches!(&action.kind,
        RestoreActionKind::WriteProjectFile { logical_path, .. }
            if logical_path.as_str() == format!("project/{LARGE_FILE}"))));
    assert_file_digest(
        &projects[1].join(LARGE_FILE),
        LARGE_FILE_BYTES,
        &large_digest,
    );
    assert_file_digest(
        &projects[0].join(LARGE_FILE),
        LARGE_FILE_BYTES,
        &large_digest,
    );
    println!("Claude media: 200000000-byte file restored from six verified chunks");
    let restored_video = std::fs::read(projects[1].join(VIDEO)).unwrap();
    assert_eq!(restored_video, VIDEO_BYTES);
    assert_eq!(mallard_domain::sha256_hex(&restored_video), media.sha256);
    let b_thread = local_thread_of_cloud(&repositories[1], &b.project.local_project_id, &cloud);
    let b_session = provider_thread_id(&repositories[1], &b.project.local_project_id, &b_thread);
    let restored = transcript_bytes(&homes[1], &projects[1], &b_session);
    assert_media_record(&restored, projects[1].join("part5").to_str().unwrap());
    assert_excludes(
        &restored,
        &[projects[0].to_string_lossy().as_bytes().to_vec()],
        "restored media transcript",
    );
    assert_detail(
        &services[1],
        &b.project.local_project_id,
        &mallard_domain::conversation_resource_id(&claude_key(), &b_thread).unwrap(),
    );
    assert!(
        canonicalize(&projects[1].join("part5").join(format!("../{VIDEO}"))).unwrap()
            == projects[1].join(VIDEO)
    );
    run_claude_with_tool(
        &mut processes,
        &provider,
        &homes[1],
        &process,
        &projects[1],
        &b_session,
        false,
        RESUME_PROMPT,
        RESUME_RESPONSE,
        "Bash",
    );
    assert_eq!(processes, 2);
    assert_media_record(
        &transcript_bytes(&homes[1], &projects[1], &b_session),
        projects[1].join("part5").to_str().unwrap(),
    );
    assert_eq!(transcript_bytes(&homes[0], &projects[0], SESSION), source);
    let sensitive = vec![
        PLACEHOLDER_API_KEY.as_bytes().to_vec(),
        INHERITED_SECRET_MARKER.as_bytes().to_vec(),
    ];
    let mut private = sensitive.clone();
    private.extend(
        projects
            .iter()
            .map(|path| path.to_string_lossy().as_bytes().to_vec()),
    );
    private.push(
        canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .as_bytes()
            .to_vec(),
    );
    assert_portable_state_is_safe(
        &services[0],
        &storage.id,
        &bundle,
        &storage_root,
        &private,
        &private,
        &[],
    );
    assert_excludes(&view.bytes, &private, "media cloud view");
    for path in [&homes[0], &homes[1], &machines[0], &machines[1], &process] {
        assert_private_tree(path, &sensitive, "media fixture private state");
    }
    aimock
        .finish(&sensitive)
        .expect("media mock must account for exactly three requests and two probes");
    println!("Claude media passed: native cwd change, portable cwd, restored video, 200 MB file, cloud view, native resume");
}

#[test]
#[ignore = "two native CLI turns against local AIMock; select --scenario media"]
fn live_claude_aimock_media_round_trip() {
    if std::env::var(LIVE_TEST_GATE).ok().as_deref() == Some("1") {
        run();
    }
}
