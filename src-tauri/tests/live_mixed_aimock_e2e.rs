//! Private, gated service worker for scripts/run-mixed-aimock-e2e.mjs.
//! Native data and independent expected results belong to the Node harness.
//! All paths are rooted in a disposable fixture. No Tauri application starts.
use mallard_application::project_sync::service::{
    ProjectSessionTimelineRequest, ProjectSyncService, PushApproval, PushReviewContext,
};
use mallard_domain::{
    BundleId, DraftProfileSelection, DraftRepositoryChoice, DraftStorageSelection,
    LocalProjectRegistration, ProjectBinding, ProviderKey, ResourceId, RestoreActionKind,
    RestorePlan, StorageId,
};
use mallard_infrastructure::{persistence::LocalStateRepository, sync_metrics};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: u64,
    operation: String,
    #[serde(default)]
    args: Value,
}

struct Context {
    repository: LocalStateRepository,
    project: LocalProjectRegistration,
    binding: ProjectBinding,
    storage_id: StorageId,
    codex_home: PathBuf,
    review: Option<PushReviewContext>,
    plan: Option<RestorePlan>,
    metrics: Vec<sync_metrics::RegisteredRoot>,
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing field {key}"))
}

fn private_directory(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(path).map_err(|_| "fixture directory missing")?;
    if !path.starts_with(root) || path == root {
        return Err("fixture escaped root".into());
    }
    let relative = path.strip_prefix(root).map_err(|_| "fixture root")?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        current.push(part);
        let metadata = std::fs::symlink_metadata(&current).map_err(|_| "fixture metadata")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("fixture directory type".into());
        }
    }
    Ok(path)
}

fn open_context(root: &Path, args: &Value) -> Result<Context, String> {
    let name = field(args, "replica")?;
    if !["a", "b", "c"].contains(&name) {
        return Err("fixture replica".into());
    }
    let machine = private_directory(root, &root.join(name).join("machine"))?;
    let project_root = private_directory(root, Path::new(field(args, "project")?))?;
    let source = args
        .get("source_replica")
        .and_then(Value::as_str)
        .unwrap_or(name);
    if !["a", "b", "c"].contains(&source) {
        return Err("fixture source replica".into());
    }
    let codex = private_directory(root, &root.join(source).join("codex"))?;
    let claude = private_directory(root, &root.join(source).join("claude"))?;
    let storage_directory = if args.get("separate_storage").and_then(Value::as_bool) == Some(true) {
        "reconciliation-storage"
    } else {
        "storage"
    };
    let storage_root = private_directory(root, &root.join(storage_directory))?;
    let repository = LocalStateRepository::from_home_dir(&machine)?;
    let storage_id = StorageId::parse(
        args.get("storage_id")
            .and_then(Value::as_str)
            .unwrap_or("storage-mixed-aimock"),
    )?;
    let mut config = repository.load_config()?;
    if !config
        .storages
        .iter()
        .any(|storage| storage.id == storage_id)
    {
        config.storages.push(
            mallard_infrastructure::storage_providers::local::local_storage_config(
                storage_id.clone(),
                "Mixed local fixture",
                &storage_root,
            ),
        );
        repository.save_config(config)?;
    }
    let mut settings = mallard_infrastructure::app_settings::load_app_settings(&repository)?;
    // The fixture expects redacted bodies and checks storage for credential leaks.
    settings.credential_redaction_enabled = true;
    if let Some(include_children) = args.get("include_children").and_then(Value::as_bool) {
        settings.push_codex_children = include_children;
        settings.push_claude_children = include_children;
    }
    if let Some(workers) = args.get("workers").and_then(Value::as_u64) {
        settings.comparison_workers = workers as u32;
    }
    mallard_infrastructure::app_settings::save_app_settings(&repository, &settings)?;
    mallard_infrastructure::app_settings::install_app_settings(&settings);
    let service = ProjectSyncService::new(&repository);
    let existing = repository.load_config()?.projects.into_iter().next();
    let (project, binding) = if let Some(project) = existing {
        let binding = repository
            .load_bindings()?
            .active_for(&project.local_project_id)
            .cloned()
            .ok_or("fixture binding missing")?;
        (project, binding)
    } else {
        let mut draft = service.create_setup_draft(&project_root)?.draft;
        for (provider, home) in [("codex", codex.clone()), ("claude", claude.clone())] {
            draft.profiles.insert(
                ProviderKey::parse(provider)?,
                DraftProfileSelection::Pending {
                    path: home.to_string_lossy().into_owned(),
                    display_name: "Mixed fixture".into(),
                },
            );
        }
        draft.storage = Some(DraftStorageSelection {
            storage_id: storage_id.clone(),
        });
        draft.repository = if let Some(bundle) = args.get("bundle") {
            DraftRepositoryChoice::Existing {
                storage_id: storage_id.clone(),
                bundle_id: BundleId::parse(field(bundle, "bundle_id")?)?,
                display_name: field(bundle, "display_name")?.to_string(),
                repository_fingerprint: bundle
                    .get("repository_fingerprint")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                mismatch_acknowledged: true,
            }
        } else {
            DraftRepositoryChoice::New
        };
        let draft = service.update_setup_draft(draft)?;
        if !service.inspect_setup_draft(&draft.draft_id)?.can_finalize {
            return Err("fixture setup blocked".into());
        }
        let detail = service.finalize_project_setup(&draft.draft_id, draft.revision)?;
        (
            detail.project,
            detail.binding.ok_or("fixture setup binding")?,
        )
    };
    let metrics = [&codex, &claude]
        .iter()
        .map(|path| sync_metrics::register_root(path))
        .collect();
    Ok(Context {
        repository,
        project,
        binding,
        storage_id,
        codex_home: codex,
        review: None,
        plan: None,
        metrics,
    })
}

fn metrics(value: sync_metrics::MetricsSnapshot) -> Value {
    json!({"directories_visited":value.directories_visited,"files_head_read":value.files_head_read,
        "transcript_bytes_read":value.transcript_bytes_read,"probe_bytes_read":value.probe_bytes_read,
        "append_bytes_read":value.append_bytes_read,"records_parsed":value.records_parsed,
        "bytes_hashed":value.bytes_hashed,"full_graph_resolves":value.full_graph_resolves,
        "selected_captures":value.selected_captures,
        "observation_reuse_hits":value.observation_reuse_hits})
}

fn usage() -> Value {
    #[cfg(unix)]
    {
        let mut value: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut value) } == 0 {
            let rss = value.ru_maxrss as u64;
            let rss = if cfg!(target_os = "macos") {
                rss
            } else {
                rss * 1024
            };
            let cpu =
                |time: libc::timeval| time.tv_sec as f64 * 1000.0 + time.tv_usec as f64 / 1000.0;
            return json!({"peak_rss_bytes":rss,"cpu_ms":cpu(value.ru_utime)+cpu(value.ru_stime)});
        }
    }
    Value::Null
}

fn perform(context: &mut Context, request: &Request) -> Result<Value, String> {
    let service = ProjectSyncService::new(&context.repository);
    let id = &context.project.local_project_id;
    let storage = &context.storage_id;
    match request.operation.as_str() {
        "conversation_file_limit" => Ok(json!(
            mallard_domain::MAX_CONVERSATION_FILE_BYTES
        )),
        "compare" => serde_json::to_value(service.compare_threads(id, storage)?)
            .map_err(|_| "comparison serialization".into()),
        "push_review" => {
            let review = service.prepare_push_review(id, storage)?;
            let result = serde_json::to_value(&review).map_err(|_| "review serialization")?;
            context.review = Some(review);
            Ok(result)
        }
        "push_apply" => {
            let review = context.review.take().ok_or("Push needs a review")?;
            let mut approval = PushApproval::recommended(&review, storage);
            if let Some(selection) = request.args.get("resource_ids").and_then(Value::as_array) {
                let ids = selection
                    .iter()
                    .map(|v| v.as_str().ok_or("selection type"))
                    .collect::<Result<BTreeSet<_>, _>>()?;
                approval.recipe.entries.retain(|key, _| {
                    ids.contains(key.as_str()) || key.as_str() == "codex:session-index"
                });
            }
            let mut phases = Vec::new();
            let result = service.push_reviewed_with_progress(
                id,
                storage,
                approval,
                &mut |_| {},
                &mut |name, elapsed| {
                    phases.push(json!({"name":name,"ms":elapsed.as_secs_f64()*1000.0}))
                },
            )?;
            Ok(json!({"result":result,"phases":phases}))
        }
        "pull_review" => {
            let plan = service.plan_bundle_restore(
                storage,
                &context.project.bundle_id,
                &context.binding,
            )?;
            let result = serde_json::to_value(&plan).map_err(|_| "plan serialization")?;
            context.plan = Some(plan);
            Ok(result)
        }
        "pull_apply" => {
            let mut plan = context.plan.take().ok_or("Pull needs a review")?;
            if request.args.get("expire").and_then(Value::as_bool) == Some(true) {
                plan.expires_at = 0;
            }
            let actions = plan
                .actions
                .iter()
                .filter(|a| !matches!(a.kind, RestoreActionKind::Manual { .. }))
                .map(|a| a.action_id.clone())
                .chain(
                    plan.conversation_review
                        .iter()
                        .filter_map(|r| r.action_id.clone()),
                )
                .collect();
            let mut phases = Vec::new();
            let result = service.apply_bundle_restore_with_progress(
                &plan,
                actions,
                &mut |_| {},
                &mut |name, elapsed| {
                    phases.push(json!({"name":name,"ms":elapsed.as_secs_f64()*1000.0}))
                },
            )?;
            Ok(json!({"result":result,"phases":phases}))
        }
        "graph" => {
            let home = &context.codex_home;
            let mut paths = std::collections::BTreeMap::new();
            for entry in walkdir::WalkDir::new(home.join("sessions"))
                .follow_links(false)
                .max_depth(8)
            {
                let entry = entry.map_err(|_| "graph walk")?;
                if entry.file_type().is_symlink() {
                    return Err("graph symlink".into());
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                let filename = entry.file_name().to_string_lossy();
                if !filename.ends_with(".jsonl") || filename.len() < 42 {
                    continue;
                }
                let id = filename[filename.len() - 42..filename.len() - 6].to_string();
                if paths.insert(id, entry.path().to_path_buf()).is_some() {
                    return Err("graph duplicate alias".into());
                }
                if paths.len() > 1000 {
                    return Err("graph size".into());
                }
            }
            let lookup = |id: &str| Ok(paths.get(id).cloned());
            let mut output = Vec::new();
            for id in request
                .args
                .get("ids")
                .and_then(Value::as_array)
                .ok_or("graph IDs")?
            {
                let id = id.as_str().ok_or("graph ID")?;
                let path = paths.get(id).ok_or("graph leaf missing")?;
                let resolved =
                    mallard_infrastructure::codex_history_graph::resolve_codex_root(path, &lookup)?;
                output.push(json!({"id":id,"parent":resolved.leaf_meta.history_base.as_ref().map(|b| &b.thread_id),
                    "segments":resolved.segments.iter().map(|s| json!({"id":s.provider_thread_id,"bytes":s.consumed_byte_len,"records":s.consumed_record_count})).collect::<Vec<_>>(),
                    "messages":resolved.chain.len(),"leaf_bytes":resolved.leaf_file_size}));
            }
            Ok(json!(output))
        }
        "bindings" => serde_json::to_value(context.repository.load_conversation_bindings()?)
            .map_err(|_| "bindings serialization".into()),
        "inspect" => {
            serde_json::to_value(service.inspect_bundle(storage, &context.project.bundle_id)?)
                .map_err(|_| "bundle serialization".into())
        }
        "activity" => serde_json::to_value(service.get_project_activity(
            id,
            Some(storage),
            None,
            None,
            Some(36500),
            false,
        )?)
        .map_err(|_| "activity serialization".into()),
        "timeline" | "timeline_first_page" => {
            let resource = ResourceId::parse(field(&request.args, "resource_id")?)?;
            let detail = service
                .get_project_session_detail(id, &resource)
                .map_err(|_| "detail failed")?;
            let mut cursor = None;
            let mut pages = Vec::new();
            let mut seen = BTreeSet::new();
            loop {
                let page = service.get_project_session_timeline_page(id, &resource, ProjectSessionTimelineRequest {
                    snapshot_id: detail.snapshot_id.clone(), cursor, limit: Some(17),
                    included_event_kinds: [
                        mallard_infrastructure::providers::SessionTimelineEventKind::Prompt,
                        mallard_infrastructure::providers::SessionTimelineEventKind::Response,
                        mallard_infrastructure::providers::SessionTimelineEventKind::IntermediateStep,
                        mallard_infrastructure::providers::SessionTimelineEventKind::Checkpoint,
                        mallard_infrastructure::providers::SessionTimelineEventKind::ToolCall,
                    ].into_iter().collect(), included_tool_categories: BTreeSet::new(), turn_id: None,
                }).map_err(|_| "timeline failed")?;
                cursor = page.next_cursor.clone();
                if let Some(next) = &cursor {
                    if !seen.insert(next.clone()) {
                        return Err("timeline cursor repeated".into());
                    }
                }
                pages.push(page);
                if cursor.is_none() || request.operation == "timeline_first_page" {
                    break;
                }
                if pages.len() > 1024 {
                    return Err("timeline page limit".into());
                }
            }
            Ok(json!({"detail":detail,"pages":pages}))
        }
        _ => Err("unknown worker operation".into()),
    }
}

#[test]
#[ignore = "private worker; use the gated mixed aimock runner"]
fn mixed_aimock_worker() {
    assert_eq!(
        std::env::var("MALLARD_RUN_MIXED_AIMOCK_E2E").as_deref(),
        Ok("1"),
        "mixed test gate"
    );
    std::panic::set_hook(Box::new(|_| eprintln!("mixed worker panic")));
    let root =
        PathBuf::from(std::env::var_os("MALLARD_MIXED_ROOT").expect("fixture root required"));
    let root = std::fs::canonicalize(root).expect("fixture root missing");
    assert_eq!(
        std::fs::read(root.join("fixture-owner")).unwrap(),
        b"mixed-aimock-v1\n"
    );
    let mut input = std::io::BufReader::new(std::io::stdin().lock());
    let mut context: Option<Context> = None;
    loop {
        let mut bytes = Vec::new();
        let length = input
            .by_ref()
            .take(1024 * 1024 + 1)
            .read_until(b'\n', &mut bytes)
            .expect("worker read");
        if length == 0 {
            break;
        }
        assert!(length <= 1024 * 1024, "worker request size");
        let request: Request = serde_json::from_slice(&bytes).expect("worker request format");
        let before = if request.operation == "open" {
            Vec::new()
        } else {
            context
                .as_ref()
                .map(|c| c.metrics.iter().map(|m| m.snapshot()).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let before_usage = usage();
        let started = Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if request.operation == "open" {
                let next = open_context(&root, &request.args)?;
                let result = json!({"project":next.project});
                context = Some(next);
                Ok(result)
            } else {
                perform(context.as_mut().ok_or("worker is not open")?, &request)
            }
        }))
        .unwrap_or_else(|_| Err("worker panic".into()));
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let counters: Vec<_> = context
            .as_ref()
            .map(|c| {
                c.metrics
                    .iter()
                    .zip(before.iter())
                    .map(|(m, b)| metrics(m.snapshot().delta_since(b)))
                    .collect()
            })
            .unwrap_or_default();
        let after_usage = usage();
        let reply = match result {
            Ok(value) => {
                json!({"id":request.id,"ok":true,"value":value,"ms":elapsed,"metrics":counters,"usage":after_usage,"cpu_before":before_usage})
            }
            Err(error) => json!({"id":request.id,"ok":false,"error":error,"ms":elapsed}),
        };
        // This private pipe can contain fixture paths. It is never exported.
        println!(
            "\nMIXED_REPLY {}",
            serde_json::to_string(&reply).expect("worker response")
        );
        std::io::stdout().flush().expect("worker flush");
    }
}
