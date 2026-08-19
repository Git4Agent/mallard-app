//! Opt-in end-to-end coverage for two isolated Codex homes synchronizing
//! through one real Mallard local-folder repository.
//!
//! This test makes twenty-five paid, authenticated Codex requests: five
//! conversation turns plus twenty old-history verification turns. It is
//! ignored by default, separately environment-gated, and intended to run as
//! the only test process so its temporary `HOME` override cannot affect other
//! tests.

mod support;

use mallard_domain::{
    ActionId, BundleId, ConversationFileRole, ConversationPlanActionType, ConversationReviewAction,
    DraftProfileSelection, DraftRepositoryChoice, DraftStorageSelection, KeepBothPlan,
    LocalConversationBinding, LocalProjectId, ProjectBinding, ProjectSetupDraft, ProviderKey,
    ResourceId, RestoreActionKind, RestorePlan, StorageConfig, StorageId, StorageKind,
    SyncScopeKey, ThreadId, MAX_CONVERSATION_FILE_BYTES,
};
use mallard_infrastructure::codex_history_graph::{resolve_codex_root, seed_codex_root};
use mallard_infrastructure::codex_paginated_projection::verify_projection_fixpoint;
use mallard_infrastructure::codex_rollout::analyze_codex_rollout_structure;
use mallard_infrastructure::codex_standalone_builder::{
    build_standalone_codex_root, StandaloneBuildMode,
};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::providers::{
    ProviderRegistry, SessionTimelineContext, SessionTimelineQuery,
};
use mallard_infrastructure::storage_core::repository_object_key;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::Metadata;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};
use tauri_app_lib::{
    ProjectDetail, ProjectOperationResult, ProjectSessionTimelineRequest, ProjectSyncService,
    PushApproval, PushDestinationPullReason, PushDestinationStatus, RestoreResult,
    SessionTimelineEventKind, SessionToolCategory, ThreadSyncComparison, ThreadSyncState,
};
use walkdir::WalkDir;

use support::catalog_flow::{run_catalog_flow, storage_snapshot, CatalogFlowInputs};

const LIVE_TEST_GATE: &str = "MALLARD_RUN_LIVE_CODEX_E2E";
const SOURCE_HOME_ENV: &str = "MALLARD_LIVE_CODEX_SOURCE_HOME";
const MODEL: &str = "gpt-5.6-luna";
const REASONING_OVERRIDE: &str = "model_reasoning_effort=\"low\"";
const MAX_AUTH_BYTES: u64 = 1024 * 1024;
const MAX_CODEX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_CODEX_OUTPUT_EVENTS: usize = 4_096;
const MAX_CODEX_SYNOPSIS_EVENTS: usize = 64;
const MAX_COMMAND_ARGUMENT_BYTES: usize = 4096;
const MAX_COMMAND_ARGUMENTS: usize = 24;
const MAX_PROMPT_BYTES: usize = 512;
const MAX_WRITE_PROMPT_BYTES: usize = 2 * 1024;
const MAX_STORAGE_OBJECT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SNAPSHOT_FILES: usize = 2_048;
const MAX_FIXTURE_FILE_BYTES: u64 = 4 * 1024;
const MAX_FIXTURE_TREE_ENTRIES: usize = 32;
const MAX_SESSION_DETAIL_EVENTS: usize = 1_024;
const CODEX_TIMEOUT: Duration = Duration::from_secs(180);
const EXPECTED_CODEX_VERSION_PREFIX: &str = "codex-cli 0.147.";
const EXPECTED_MODEL_REQUESTS: usize = 25;

const P0: &str = "019f7800-0000-7000-8000-000000000001";
const P1: &str = "019f7800-0000-7000-8000-000000000002";
const P2: &str = "019f7800-0000-7000-8000-000000000003";
const P3: &str = "019f7800-0000-7000-8000-000000000004";
const P4: &str = "019f7800-0000-7000-8000-000000000005";

const INSIDE_ALPHA_PATH: &str = "generated/session-paths/inside-alpha.txt";
const INSIDE_BETA_PATH: &str = "generated/session-paths/nested/inside-beta.json";
const OUTSIDE_ALPHA_PATH: &str = "outside-alpha.txt";
const OUTSIDE_BETA_PATH: &str = "nested/outside-beta.json";
const INSIDE_ALPHA_BYTES: &[u8] = b"MALLARD_IN_PROJECT_ALPHA\n";
const INSIDE_BETA_BYTES: &[u8] = b"{\"fixture\":\"MALLARD_IN_PROJECT_BETA\",\"version\":1}\n";
const OUTSIDE_ALPHA_BYTES: &[u8] = b"MALLARD_OUTSIDE_ALPHA\n";
const OUTSIDE_BETA_BYTES: &[u8] = b"{\"fixture\":\"MALLARD_OUTSIDE_BETA\",\"version\":1}\n";
const IN_PROJECT_FIXTURES: [(&str, &[u8]); 2] = [
    (INSIDE_ALPHA_PATH, INSIDE_ALPHA_BYTES),
    (INSIDE_BETA_PATH, INSIDE_BETA_BYTES),
];
const OUTSIDE_PROJECT_FIXTURES: [(&str, &[u8]); 2] = [
    (OUTSIDE_ALPHA_PATH, OUTSIDE_ALPHA_BYTES),
    (OUTSIDE_BETA_PATH, OUTSIDE_BETA_BYTES),
];

fn redacted<T, E>(result: Result<T, E>, message: &'static str) -> T {
    match result {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

fn codex_binary() -> PathBuf {
    redacted(
        mallard_infrastructure::codex_binary::find_binary("codex"),
        "the live test requires an installed Codex CLI",
    )
}

fn assert_supported_codex_version() {
    let output = Command::new(codex_binary())
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .expect("query the Codex CLI version");
    assert!(
        output.status.success(),
        "cannot query the Codex CLI version"
    );
    assert!(
        output.stdout.len() <= 256 && output.stderr.len() <= 1024,
        "Codex version output exceeds the test safety bound"
    );
    let version = String::from_utf8(output.stdout).expect("Codex version must be UTF-8");
    assert!(
        version.trim().starts_with(EXPECTED_CODEX_VERSION_PREFIX),
        "the live fixture requires Codex 0.147.x; update the current-format writer before running against another version"
    );
}

struct EnvironmentGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvironmentGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvironmentGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthFileState {
    len: u64,
    modified: Option<SystemTime>,
    readonly: bool,
    digest: [u8; 32],
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    mode: u32,
    #[cfg(unix)]
    uid: u32,
    #[cfg(unix)]
    gid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FixtureFileState {
    len: u64,
    modified: Option<SystemTime>,
    readonly: bool,
    digest: [u8; 32],
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    mode: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodexRunAccess<'a> {
    ReadOnly,
    WorkspaceWrite { additional_dir: &'a Path },
}

fn regular_auth_metadata(path: &Path) -> Metadata {
    let metadata = std::fs::symlink_metadata(path).expect("live Codex auth.json is unavailable");
    assert!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "live Codex auth.json must be a regular file"
    );
    assert!(
        metadata.len() <= MAX_AUTH_BYTES,
        "live Codex auth.json exceeds the test safety bound"
    );
    metadata
}

fn read_bounded_file(path: &Path, maximum: u64, label: &str) -> Vec<u8> {
    let metadata =
        std::fs::symlink_metadata(path).unwrap_or_else(|_| panic!("{label} is unreadable"));
    assert!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "{label} must be a regular file"
    );
    assert!(
        metadata.len() <= maximum,
        "{label} exceeds its safety bound"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::fs::File::open(path)
        .unwrap_or_else(|_| panic!("{label} is unreadable"))
        .take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)
        .unwrap_or_else(|_| panic!("{label} is unreadable"));
    assert!(
        bytes.len() as u64 <= maximum,
        "{label} exceeds its safety bound"
    );
    bytes
}

fn auth_file_state(path: &Path) -> AuthFileState {
    let metadata = regular_auth_metadata(path);
    let bytes = read_bounded_file(path, MAX_AUTH_BYTES, "Codex authentication file");
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt as _;

    AuthFileState {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        readonly: metadata.permissions().readonly(),
        digest: Sha256::digest(bytes).into(),
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
        #[cfg(unix)]
        mode: metadata.mode(),
        #[cfg(unix)]
        uid: metadata.uid(),
        #[cfg(unix)]
        gid: metadata.gid(),
    }
}

fn clean_fixture_relative_path(value: &str) -> &Path {
    let path = Path::new(value);
    assert!(
        !path.as_os_str().is_empty()
            && !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "live path fixture has an unsafe relative path"
    );
    path
}

fn assert_no_follow_directory_chain(root: &Path, relative: &Path) {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            panic!("fixture directory chain has an unsafe component");
        };
        current.push(component);
        let metadata =
            std::fs::symlink_metadata(&current).expect("inspect fixture directory chain");
        assert!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "fixture directory chain contains a non-directory or symlink"
        );
    }
}

fn prepare_fixture_parents(root: &Path, fixtures: &[(&str, &[u8])]) {
    let root_metadata = std::fs::symlink_metadata(root).expect("inspect live path fixture root");
    assert!(
        root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
        "live path fixture root must be a regular directory"
    );
    let canonical_root = std::fs::canonicalize(root).expect("canonicalize live path fixture root");
    for (relative, _) in fixtures {
        let relative = clean_fixture_relative_path(relative);
        let parent = relative.parent().expect("fixture path has a parent");
        std::fs::create_dir_all(root.join(parent)).expect("create live path fixture parent");
        assert_no_follow_directory_chain(root, parent);
        let canonical_parent =
            std::fs::canonicalize(root.join(parent)).expect("canonicalize fixture parent");
        assert!(
            canonical_parent.starts_with(&canonical_root),
            "live path fixture parent escaped its root"
        );
    }
}

fn fixture_file_state(path: &Path, expected: &[u8]) -> FixtureFileState {
    let metadata = std::fs::symlink_metadata(path).expect("live path fixture is unavailable");
    assert!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "live path fixture must be a no-follow regular file"
    );
    assert!(
        metadata.len() <= MAX_FIXTURE_FILE_BYTES,
        "live path fixture exceeds its safety bound"
    );
    let bytes = read_bounded_file(path, MAX_FIXTURE_FILE_BYTES, "live path fixture");
    assert!(bytes == expected, "live path fixture has unexpected bytes");
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt as _;
    #[cfg(unix)]
    assert_eq!(
        metadata.mode() & 0o777,
        0o644,
        "live path fixture has an unexpected Unix mode"
    );
    FixtureFileState {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        readonly: metadata.permissions().readonly(),
        digest: Sha256::digest(bytes).into(),
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
        #[cfg(unix)]
        mode: metadata.mode() & 0o777,
    }
}

fn fixture_file_states(
    root: &Path,
    fixtures: &[(&str, &[u8])],
) -> BTreeMap<PathBuf, FixtureFileState> {
    fixtures
        .iter()
        .map(|(relative, expected)| {
            let relative = clean_fixture_relative_path(relative).to_path_buf();
            let state = fixture_file_state(&root.join(&relative), expected);
            (relative, state)
        })
        .collect()
}

fn assert_exact_fixture_tree(root: &Path, fixtures: &[(&str, &[u8])]) {
    let expected_files = fixtures
        .iter()
        .map(|(relative, _)| clean_fixture_relative_path(relative).to_path_buf())
        .collect::<BTreeSet<_>>();
    let mut expected_directories = BTreeSet::new();
    for relative in &expected_files {
        let mut parent = relative.parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            expected_directories.insert(directory.to_path_buf());
            parent = directory.parent();
        }
    }

    let mut found_files = BTreeSet::new();
    let mut entries = 0_usize;
    for entry in WalkDir::new(root).follow_links(false).max_depth(8) {
        let entry = redacted(entry, "cannot inspect live path fixture tree");
        entries = entries.saturating_add(1);
        assert!(
            entries <= MAX_FIXTURE_TREE_ENTRIES,
            "live path fixture tree exceeds its entry bound"
        );
        assert!(
            !entry.file_type().is_symlink(),
            "live path fixture tree contains a symlink"
        );
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("fixture entry remains below its root");
        if relative.as_os_str().is_empty() {
            assert!(entry.file_type().is_dir());
        } else if entry.file_type().is_dir() {
            assert!(
                expected_directories.contains(relative),
                "live path fixture tree contains an unexpected directory"
            );
        } else if entry.file_type().is_file() {
            assert!(
                expected_files.contains(relative),
                "live path fixture tree contains an unexpected file"
            );
            found_files.insert(relative.to_path_buf());
        } else {
            panic!("live path fixture tree contains a special file");
        }
    }
    assert_eq!(
        found_files, expected_files,
        "live path fixture tree is incomplete"
    );
    let _ = fixture_file_states(root, fixtures);
}

fn copy_in_project_fixtures(source_root: &Path, target_root: &Path) {
    let source_root = std::fs::canonicalize(source_root).expect("canonicalize source project");
    let target_root = std::fs::canonicalize(target_root).expect("canonicalize target project");
    assert_ne!(source_root, target_root, "project replicas share one root");
    assert!(
        !source_root.starts_with(&target_root) && !target_root.starts_with(&source_root),
        "project replica roots overlap"
    );

    for (relative, expected) in IN_PROJECT_FIXTURES {
        let relative = clean_fixture_relative_path(relative);
        let source = source_root.join(relative);
        let source_state = fixture_file_state(&source, expected);
        let target = target_root.join(relative);
        assert!(!target.exists(), "fixture copy would overwrite a target");
        let target_parent = target.parent().expect("fixture target has a parent");
        std::fs::create_dir_all(target_parent).expect("create copied fixture parent");
        assert_no_follow_directory_chain(
            &target_root,
            target_parent
                .strip_prefix(&target_root)
                .expect("copied fixture parent remains below target"),
        );
        let canonical_parent =
            std::fs::canonicalize(target_parent).expect("canonicalize copied fixture parent");
        assert!(
            canonical_parent.starts_with(&target_root),
            "copied fixture parent escaped the target project"
        );
        std::fs::copy(&source, &target).expect("copy in-project fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(source_state.mode))
                .expect("preserve copied fixture mode");
        }
        let target_state = fixture_file_state(&target, expected);
        assert_eq!(target_state.len, source_state.len);
        assert_eq!(target_state.digest, source_state.digest);
        assert_eq!(target_state.readonly, source_state.readonly);
        #[cfg(unix)]
        {
            assert_eq!(target_state.mode, source_state.mode);
            assert_ne!(
                (target_state.device, target_state.inode),
                (source_state.device, source_state.inode),
                "fixture copy reused the source file identity"
            );
        }
    }
    assert_exact_fixture_tree(&target_root, &IN_PROJECT_FIXTURES);
}

fn canonical_fixture_paths(root: &Path, fixtures: &[(&str, &[u8])]) -> Vec<PathBuf> {
    let root = std::fs::canonicalize(root).expect("canonicalize live path fixture root");
    fixtures
        .iter()
        .map(|(relative, _)| root.join(clean_fixture_relative_path(relative)))
        .collect()
}

fn fixture_path_spellings(root: &Path, relative: &str) -> BTreeSet<PathBuf> {
    let relative = clean_fixture_relative_path(relative);
    let canonical_root =
        std::fs::canonicalize(root).expect("canonicalize live path fixture root spelling");
    [root.join(relative), canonical_root.join(relative)]
        .into_iter()
        .collect()
}

fn revision_one_file_prompt(project_root: &Path, outside_root: &Path) -> String {
    let mut requirements = Vec::new();
    for (root, fixtures) in [
        (project_root, IN_PROJECT_FIXTURES.as_slice()),
        (outside_root, OUTSIDE_PROJECT_FIXTURES.as_slice()),
    ] {
        assert!(root.is_absolute(), "prompt fixture root must be absolute");
        for (relative, contents) in fixtures {
            let path = root.join(clean_fixture_relative_path(relative));
            let path = path
                .to_str()
                .expect("live path fixture must have a UTF-8 path");
            let contents = std::str::from_utf8(contents).expect("fixture bytes must be UTF-8");
            requirements.push(format!(
                "path {} with exact UTF-8 bytes {}",
                serde_json::to_string(path).expect("quote fixture path"),
                serde_json::to_string(contents).expect("quote fixture contents")
            ));
        }
    }
    let prompt = format!(
        "LIVE_USER_R1. Make exactly one code-mode tool call. In that call, invoke await tools.apply_patch(...) exactly once with one patch that creates exactly these four new files and makes no other filesystem changes: {}. Do not invoke any other nested tool, read files, run shell commands, or access the network. After the patch succeeds, reply with exactly: MALLARD_BASE",
        requirements.join("; ")
    );
    assert!(
        prompt.len() <= MAX_WRITE_PROMPT_BYTES && !prompt.chars().any(char::is_control),
        "revision-1 file prompt violates the argument bound"
    );
    prompt
}

fn copy_auth_only(source_home: &Path, target_home: &Path) -> AuthFileState {
    let source = source_home.join("auth.json");
    regular_auth_metadata(&source);
    std::fs::create_dir_all(target_home).expect("create isolated Codex home");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(target_home, std::fs::Permissions::from_mode(0o700))
            .expect("secure isolated Codex home");
    }
    let target = target_home.join("auth.json");
    std::fs::copy(source, &target).expect("copy live Codex authentication");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
            .expect("secure copied Codex authentication");
    }
    auth_file_state(&target)
}

fn source_session_files(source_home: &Path) -> BTreeSet<PathBuf> {
    let sessions = source_home.join("sessions");
    if !sessions.exists() {
        return BTreeSet::new();
    }
    let mut files = BTreeSet::new();
    for entry in WalkDir::new(&sessions).follow_links(false).max_depth(8) {
        let entry = redacted(entry, "cannot inspect source Codex sessions");
        if entry.file_type().is_file() {
            files.insert(
                entry
                    .path()
                    .strip_prefix(&sessions)
                    .expect("source session remains below its root")
                    .to_path_buf(),
            );
            assert!(
                files.len() <= 50_000,
                "source Codex session inventory exceeds the test safety bound"
            );
        }
    }
    files
}

fn parse_codex_output_events(stdout: &[u8]) -> Result<Vec<Value>, &'static str> {
    let stdout = std::str::from_utf8(stdout).map_err(|_| "stdout_not_utf8")?;
    let mut events = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        if events.len() >= MAX_CODEX_OUTPUT_EVENTS {
            return Err("stdout_event_limit");
        }
        let event = serde_json::from_str::<Value>(line).map_err(|_| "malformed_stdout_jsonl")?;
        if !event.is_object() {
            return Err("non_object_stdout_event");
        }
        events.push(event);
    }
    if events.is_empty() {
        return Err("empty_stdout_jsonl");
    }
    Ok(events)
}

fn diagnostic_atom(value: Option<&Value>) -> String {
    const SAFE_ATOMS: &[&str] = &[
        "agent_message",
        "cancelled",
        "command_execution",
        "completed",
        "error",
        "failed",
        "file_change",
        "in_progress",
        "item.completed",
        "item.started",
        "item.updated",
        "mcp_tool_call",
        "reasoning",
        "success",
        "thread.started",
        "todo_list",
        "turn.completed",
        "turn.failed",
        "turn.started",
        "web_search",
    ];
    match value.and_then(Value::as_str) {
        Some(value) if SAFE_ATOMS.contains(&value) => value.to_string(),
        Some(_) => "other".to_string(),
        None => "-".to_string(),
    }
}

fn codex_event_synopsis(events: &[Value]) -> String {
    let start = events.len().saturating_sub(MAX_CODEX_SYNOPSIS_EVENTS);
    let mut synopsis = format!("count={}", events.len());
    if start > 0 {
        synopsis.push_str(&format!(",omitted={start}"));
    }
    for event in &events[start..] {
        let item = event.get("item");
        let error = event.get("error");
        synopsis.push_str(&format!(
            ";type={},item={},status={},code={}",
            diagnostic_atom(event.get("type")),
            diagnostic_atom(item.and_then(|item| item.get("type"))),
            diagnostic_atom(
                event
                    .get("status")
                    .or_else(|| item.and_then(|item| item.get("status")))
            ),
            diagnostic_atom(
                event
                    .get("code")
                    .or_else(|| error.and_then(|error| error.get("code")))
            ),
        ));
    }
    synopsis
}

fn codex_stderr_synopsis(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    format!(
        "bytes={},lines={},sandbox={},permission_denied={},operation_not_permitted={},approval={},error={}",
        stderr.len(),
        text.lines().count(),
        text.contains("sandbox"),
        text.contains("permission denied"),
        text.contains("operation not permitted"),
        text.contains("approval"),
        text.contains("error"),
    )
}

fn run_codex(
    request_count: &mut usize,
    codex_home: &Path,
    process_home: &Path,
    project_root: &Path,
    access: CodexRunAccess<'_>,
    resume_thread_id: Option<&str>,
    prompt: &str,
) -> String {
    let prompt_limit = match access {
        CodexRunAccess::ReadOnly => MAX_PROMPT_BYTES,
        CodexRunAccess::WorkspaceWrite { .. } => MAX_WRITE_PROMPT_BYTES,
    };
    assert!(
        !prompt.is_empty() && prompt.len() <= prompt_limit && !prompt.chars().any(char::is_control),
        "live Codex prompt violates the test argument bound"
    );
    if let Some(thread_id) = resume_thread_id {
        assert!(
            !thread_id.is_empty()
                && thread_id.len() <= 128
                && !thread_id.chars().any(char::is_control),
            "live Codex resume ID violates the test argument bound"
        );
    }

    let mut arguments = [
        "--ask-for-approval",
        "never",
        "exec",
        "--json",
        "--ignore-user-config",
        "--ignore-rules",
        "--skip-git-repo-check",
        "--model",
        MODEL,
        "--config",
        REASONING_OVERRIDE,
        "--cd",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    arguments.push(project_root.as_os_str().to_owned());
    match access {
        CodexRunAccess::ReadOnly => {
            arguments.extend([OsString::from("--sandbox"), OsString::from("read-only")]);
        }
        CodexRunAccess::WorkspaceWrite { additional_dir } => {
            let project_metadata =
                std::fs::symlink_metadata(project_root).expect("inspect live Codex project root");
            let additional_metadata = std::fs::symlink_metadata(additional_dir)
                .expect("inspect live Codex additional directory");
            assert!(
                project_metadata.is_dir()
                    && additional_metadata.is_dir()
                    && !project_metadata.file_type().is_symlink()
                    && !additional_metadata.file_type().is_symlink(),
                "write-capable Codex roots must be no-follow directories"
            );
            let canonical_project =
                std::fs::canonicalize(project_root).expect("canonicalize live Codex project");
            let canonical_additional = std::fs::canonicalize(additional_dir)
                .expect("canonicalize live Codex additional directory");
            assert_eq!(
                canonical_project.parent(),
                canonical_additional.parent(),
                "write-capable Codex roots must be exact siblings"
            );
            assert!(
                !canonical_project.starts_with(&canonical_additional)
                    && !canonical_additional.starts_with(&canonical_project),
                "write-capable Codex roots overlap"
            );
            assert_eq!(
                canonical_additional, additional_dir,
                "write-capable Codex additional directory must use its canonical spelling"
            );
            arguments.extend([
                OsString::from("--sandbox"),
                OsString::from("workspace-write"),
                OsString::from("--add-dir"),
                additional_dir.as_os_str().to_owned(),
            ]);
        }
    }
    if let Some(thread_id) = resume_thread_id {
        arguments.extend([OsString::from("resume"), OsString::from(thread_id)]);
    }
    arguments.push(OsString::from(prompt));
    assert!(
        arguments.len() <= MAX_COMMAND_ARGUMENTS
            && arguments
                .iter()
                .all(|argument| argument.as_encoded_bytes().len() <= MAX_COMMAND_ARGUMENT_BYTES),
        "live Codex command violates the argv safety bound"
    );

    *request_count = request_count.saturating_add(1);
    let mut command = Command::new(codex_binary());
    let mut child = command
        .args(arguments)
        .current_dir(project_root)
        .env("CODEX_HOME", codex_home)
        .env("HOME", process_home)
        .env("USERPROFILE", process_home)
        .env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_API_KEY")
        .env_remove("AZURE_OPENAI_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the Codex CLI");
    let stdout = child.stdout.take().expect("capture Codex JSON output");
    let stderr = child.stderr.take().expect("capture Codex stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take((MAX_CODEX_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take((MAX_CODEX_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });

    let deadline = Instant::now() + CODEX_TIMEOUT;
    loop {
        if child.try_wait().expect("wait for the Codex CLI").is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            panic!("the live Codex request exceeded its timeout");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let status = child.wait().expect("finish the Codex CLI");
    let stdout = stdout_reader
        .join()
        .expect("Codex stdout reader panicked")
        .expect("collect Codex stdout");
    let stderr = stderr_reader
        .join()
        .expect("Codex stderr reader panicked")
        .expect("collect Codex stderr");
    assert!(
        stdout.len() <= MAX_CODEX_OUTPUT_BYTES && stderr.len() <= MAX_CODEX_OUTPUT_BYTES,
        "the live Codex response exceeded its output bound"
    );
    let stderr_synopsis = codex_stderr_synopsis(&stderr);
    let events = parse_codex_output_events(&stdout).unwrap_or_else(|reason| {
        panic!("the live Codex response was invalid ({reason}); stderr: {stderr_synopsis}")
    });
    let event_synopsis = codex_event_synopsis(&events);
    assert!(
        status.success(),
        "the live Codex request failed; events: {event_synopsis}; stderr: {stderr_synopsis}"
    );
    let thread_ids = events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("thread.started"))
        .filter_map(|event| event.get("thread_id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(
        thread_ids.len(),
        1,
        "Codex did not report exactly one started thread; events: {event_synopsis}; stderr: {stderr_synopsis}"
    );
    let provider_thread_id = thread_ids[0].to_string();

    if matches!(access, CodexRunAccess::WorkspaceWrite { .. }) {
        let final_messages = events
            .iter()
            .filter(|event| event.get("type").and_then(Value::as_str) == Some("item.completed"))
            .filter_map(|event| event.get("item"))
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("agent_message"))
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(
            final_messages.len() == 1 && final_messages[0] == "MALLARD_BASE",
            "revision-1 Codex output lacked its single exact final marker; events: {event_synopsis}; stderr: {stderr_synopsis}"
        );
        let rollout = rollout_lookup(codex_home, &provider_thread_id)
            .unwrap_or_else(|_| {
                panic!(
                    "revision-1 rollout discovery failed; events: {event_synopsis}; stderr: {stderr_synopsis}"
                )
            })
            .unwrap_or_else(|| {
                panic!(
                    "revision-1 rollout was not materialized; events: {event_synopsis}; stderr: {stderr_synopsis}"
                )
            });
        let rollout = read_bounded_file(
            &rollout,
            MAX_CONVERSATION_FILE_BYTES,
            "revision-1 Codex rollout",
        );
        let patch_keys = try_patch_apply_change_keys(&rollout).unwrap_or_else(|reason| {
            panic!(
                "revision-1 patch evidence failed ({reason}); events: {event_synopsis}; stderr: {stderr_synopsis}"
            )
        });
        assert!(
            local_patch_change_key_candidates(
                project_root,
                match access {
                    CodexRunAccess::WorkspaceWrite { additional_dir } => additional_dir,
                    CodexRunAccess::ReadOnly => unreachable!(),
                },
            )
            .iter()
            .any(|expected| expected == &patch_keys),
            "revision-1 patch keys did not exactly match the requested fixtures; events: {event_synopsis}; stderr: {stderr_synopsis}"
        );
    }

    provider_thread_id
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum HistoryRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct HistoryMarker {
    role: HistoryRole,
    marker: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryReport {
    history: Vec<HistoryMarker>,
}

#[derive(Clone)]
struct HistoryBaseFixture {
    thread_id: String,
    end_ordinal_exclusive: u64,
    end_byte_offset: u64,
}

#[derive(Clone)]
struct GraphNode {
    provider_thread_id: String,
    path: PathBuf,
    expected: Vec<HistoryMarker>,
}

struct PaginatedGraph {
    nodes: BTreeMap<&'static str, GraphNode>,
}

fn marker(role: HistoryRole, value: &str) -> HistoryMarker {
    HistoryMarker {
        role,
        marker: value.to_string(),
    }
}

fn marker_turn(user: &str, assistant: &str) -> Vec<HistoryMarker> {
    vec![
        marker(HistoryRole::User, user),
        marker(HistoryRole::Assistant, assistant),
    ]
}

fn fixture_uuid(value: u64) -> String {
    format!("019f7800-0000-7000-8000-{value:012x}")
}

fn session_meta_record(
    provider_thread_id: &str,
    cwd: &Path,
    history_base: Option<&HistoryBaseFixture>,
) -> Value {
    json!({
        "timestamp": "2026-08-18T00:00:00Z",
        "type": "session_meta",
        "payload": {
            "id": provider_thread_id,
            "session_id": provider_thread_id,
            "timestamp": "2026-08-18T00:00:00Z",
            "cwd": cwd.to_string_lossy(),
            "originator": "codex_exec",
            "cli_version": "0.147.0",
            "source": "exec",
            "model_provider": "openai",
            "base_instructions": null,
            "history_mode": "paginated",
            "history_base": history_base.map(|base| json!({
                "thread_id": base.thread_id,
                "end_ordinal_exclusive": base.end_ordinal_exclusive,
                "end_byte_offset": base.end_byte_offset,
            })),
        },
    })
}

fn turn_records(
    provider_thread_id: &str,
    turn_id: &str,
    user_item_id: &str,
    assistant_item_id: &str,
    parent_message_id: Option<&str>,
    user_marker: &str,
    assistant_marker: &str,
) -> Vec<Value> {
    vec![
        json!({
            "timestamp": "2026-08-18T00:00:01Z",
            "type": "event_msg",
            "payload": {
                "type": "task_started",
                "turn_id": turn_id,
                "model_context_window": 114000,
                "collaboration_mode_kind": "default",
            },
        }),
        json!({
            "timestamp": "2026-08-18T00:00:02Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": user_item_id,
                "parent_id": parent_message_id,
                "role": "user",
                "content": [{"type": "input_text", "text": user_marker}],
            },
        }),
        json!({
            "timestamp": "2026-08-18T00:00:02Z",
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "thread_id": provider_thread_id,
                "turn_id": turn_id,
                "item": {
                    "type": "UserMessage",
                    "id": user_item_id,
                    "content": [{"type": "text", "text": user_marker, "text_elements": []}],
                },
                "completed_at_ms": 1787011202000_i64,
            },
        }),
        json!({
            "timestamp": "2026-08-18T00:00:03Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": assistant_item_id,
                "parent_id": user_item_id,
                "role": "assistant",
                "content": [{"type": "output_text", "text": assistant_marker}],
                "phase": "final_answer",
            },
        }),
        json!({
            "timestamp": "2026-08-18T00:00:03Z",
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "thread_id": provider_thread_id,
                "turn_id": turn_id,
                "item": {
                    "type": "AgentMessage",
                    "id": assistant_item_id,
                    "content": [{"type": "Text", "text": assistant_marker}],
                    "phase": "final_answer",
                },
                "completed_at_ms": 1787011203000_i64,
            },
        }),
        json!({
            "timestamp": "2026-08-18T00:00:04Z",
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": turn_id,
                "last_agent_message": assistant_marker,
            },
        }),
    ]
}

fn encode_rollout_records(mut records: Vec<Value>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (ordinal, record) in records.iter_mut().enumerate() {
        record
            .as_object_mut()
            .expect("rollout record must be an object")
            .insert("ordinal".to_string(), json!(ordinal as u64));
        serde_json::to_writer(&mut bytes, record).expect("serialize paginated rollout record");
        bytes.push(b'\n');
    }
    assert!(
        bytes.len() as u64 <= MAX_CONVERSATION_FILE_BYTES,
        "generated rollout exceeds the transcript safety bound"
    );
    bytes
}

fn rollout_path(codex_home: &Path, provider_thread_id: &str) -> PathBuf {
    let suffix = format!("-{provider_thread_id}.jsonl");
    let mut matches = Vec::new();
    for root_name in ["sessions", "archived_sessions"] {
        let root = codex_home.join(root_name);
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).follow_links(false).max_depth(8) {
            let entry = redacted(entry, "cannot inspect isolated Codex rollouts");
            assert!(
                !entry.file_type().is_symlink(),
                "isolated Codex session tree contains a symlink"
            );
            if entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(&suffix))
            {
                matches.push(entry.path().to_path_buf());
            }
        }
    }
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one rollout with the provider thread ID"
    );
    matches.pop().expect("checked one rollout match")
}

fn rollout_lookup(codex_home: &Path, thread_id: &str) -> Result<Option<PathBuf>, String> {
    let suffix = format!("-{thread_id}.jsonl");
    let mut match_path = None;
    let mut visited = 0_usize;
    for root_name in ["sessions", "archived_sessions"] {
        let root = codex_home.join(root_name);
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).follow_links(false).max_depth(8) {
            let entry = entry.map_err(|_| "cannot inspect Codex rollout lookup".to_string())?;
            visited += 1;
            if visited > 4_096 {
                return Err("Codex rollout lookup exceeded its entry bound".to_string());
            }
            if entry.file_type().is_symlink() {
                return Err("Codex rollout lookup encountered a symlink".to_string());
            }
            if entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(&suffix))
                && match_path.replace(entry.path().to_path_buf()).is_some()
            {
                return Err("Codex rollout lookup found duplicate identities".to_string());
            }
        }
    }
    Ok(match_path)
}

fn validate_paginated_rollout(
    codex_home: &Path,
    node: &GraphNode,
    expected_segments: &[&str],
    project_root: &Path,
) {
    let bytes = read_bounded_file(
        &node.path,
        MAX_CONVERSATION_FILE_BYTES,
        "generated paginated rollout",
    );
    let structure = analyze_codex_rollout_structure(&bytes, Some(&node.provider_thread_id))
        .expect("generated paginated rollout has invalid structure");
    assert_eq!(structure.current.id, node.provider_thread_id);
    assert_eq!(structure.current.history_mode.as_deref(), Some("paginated"));
    let lookup = |thread_id: &str| rollout_lookup(codex_home, thread_id);
    let resolved = resolve_codex_root(&node.path, &lookup)
        .expect("production graph resolver rejected generated paginated rollout");
    assert_eq!(
        resolved
            .segments
            .iter()
            .map(|segment| segment.provider_thread_id.as_str())
            .collect::<Vec<_>>(),
        expected_segments
    );
    assert_eq!(resolved.chain.len(), node.expected.len());
    for pair in resolved.chain.windows(2) {
        assert_eq!(
            pair[1].parent_message_id.as_ref(),
            Some(&pair[0].message_id),
            "generated dependency graph broke message ancestry"
        );
    }
    let seeded = seed_codex_root(&resolved, &bytes, None)
        .expect("production root seeding rejected generated paginated rollout");
    assert_eq!(
        seeded.mapping.consumed_record_ordinal,
        structure.record_count as u64
    );
    let artifact = build_standalone_codex_root(
        &resolved,
        &StandaloneBuildMode::Encode {
            root: project_root.to_string_lossy().into_owned(),
            canonical_root: std::fs::canonicalize(project_root)
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
        },
        None,
    )
    .expect("standalone production builder rejected generated paginated rollout");
    assert_eq!(artifact.message_count as usize, node.expected.len());
    let records = artifact
        .root_bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<Value>(line).expect("parse standalone record"))
        .collect::<Vec<_>>();
    assert_eq!(
        records
            .iter()
            .enumerate()
            .map(|(index, record)| (index as u64, record["ordinal"].as_u64()))
            .collect::<Vec<_>>(),
        records
            .iter()
            .enumerate()
            .map(|(index, _)| (index as u64, Some(index as u64)))
            .collect::<Vec<_>>()
    );
    assert!(records[0]["payload"]["history_base"].is_null());
    verify_projection_fixpoint(&records[1..], &node.provider_thread_id)
        .expect("standalone paginated projection is not a fixpoint");
}

fn write_graph_node(
    codex_home: &Path,
    provider_thread_id: &str,
    title: &str,
    records: Vec<Value>,
    expected: Vec<HistoryMarker>,
) -> GraphNode {
    let session_dir = codex_home.join("sessions/2026/08/18");
    std::fs::create_dir_all(&session_dir).expect("create paginated session directory");
    let path = session_dir.join(format!(
        "rollout-2026-08-18T00-00-00-{provider_thread_id}.jsonl"
    ));
    assert!(!path.exists(), "generated rollout identity already exists");
    std::fs::write(&path, encode_rollout_records(records)).expect("write paginated rollout");
    let index_path = codex_home.join("session_index.jsonl");
    let mut index = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(index_path)
        .expect("open Codex session index");
    serde_json::to_writer(
        &mut index,
        &json!({
            "id": provider_thread_id,
            "thread_name": title,
            "updated_at": "2026-08-18T00:00:04Z",
        }),
    )
    .expect("write Codex session index row");
    index.write_all(b"\n").expect("finish Codex index row");
    GraphNode {
        provider_thread_id: provider_thread_id.to_string(),
        path,
        expected,
    }
}

fn exact_cutoff(thread_id: &str, bytes: &[u8]) -> HistoryBaseFixture {
    assert_eq!(bytes.last(), Some(&b'\n'));
    HistoryBaseFixture {
        thread_id: thread_id.to_string(),
        end_ordinal_exclusive: bytes.iter().filter(|byte| **byte == b'\n').count() as u64,
        end_byte_offset: bytes.len() as u64,
    }
}

fn write_paginated_graph(codex_home: &Path, project_root: &Path) -> PaginatedGraph {
    let mut nodes = BTreeMap::new();

    let mut p0_turn1 = vec![session_meta_record(P0, project_root, None)];
    p0_turn1.extend(turn_records(
        P0,
        &fixture_uuid(0x101),
        &fixture_uuid(0x102),
        &fixture_uuid(0x103),
        None,
        "GRAPH_USER_BASE_1",
        "GRAPH_BASE_1",
    ));
    let p0_turn1_bytes = encode_rollout_records(p0_turn1.clone());
    let p0_turn1_cutoff = exact_cutoff(P0, &p0_turn1_bytes);
    let mut p0_records = p0_turn1;
    p0_records.extend(turn_records(
        P0,
        &fixture_uuid(0x104),
        &fixture_uuid(0x105),
        &fixture_uuid(0x106),
        Some(&fixture_uuid(0x103)),
        "GRAPH_USER_BASE_2",
        "GRAPH_BASE_2",
    ));
    let mut p0_expected = marker_turn("GRAPH_USER_BASE_1", "GRAPH_BASE_1");
    p0_expected.extend(marker_turn("GRAPH_USER_BASE_2", "GRAPH_BASE_2"));
    let p0 = write_graph_node(
        codex_home,
        P0,
        "Mallard graph shared root",
        p0_records,
        p0_expected,
    );
    let p0_complete_cutoff = exact_cutoff(
        P0,
        &read_bounded_file(&p0.path, MAX_CONVERSATION_FILE_BYTES, "P0 rollout"),
    );

    let mut p1_records = vec![session_meta_record(
        P1,
        project_root,
        Some(&p0_turn1_cutoff),
    )];
    p1_records.extend(turn_records(
        P1,
        &fixture_uuid(0x111),
        &fixture_uuid(0x112),
        &fixture_uuid(0x113),
        Some(&fixture_uuid(0x103)),
        "GRAPH_USER_LEFT_PARENT",
        "GRAPH_LEFT_PARENT",
    ));
    let mut p1_expected = marker_turn("GRAPH_USER_BASE_1", "GRAPH_BASE_1");
    p1_expected.extend(marker_turn("GRAPH_USER_LEFT_PARENT", "GRAPH_LEFT_PARENT"));
    let p1 = write_graph_node(
        codex_home,
        P1,
        "Mallard graph left parent",
        p1_records,
        p1_expected,
    );
    let p1_complete_cutoff = exact_cutoff(
        P1,
        &read_bounded_file(&p1.path, MAX_CONVERSATION_FILE_BYTES, "P1 rollout"),
    );

    let mut p2_records = vec![session_meta_record(
        P2,
        project_root,
        Some(&p0_complete_cutoff),
    )];
    p2_records.extend(turn_records(
        P2,
        &fixture_uuid(0x121),
        &fixture_uuid(0x122),
        &fixture_uuid(0x123),
        Some(&fixture_uuid(0x106)),
        "GRAPH_USER_RIGHT_PARENT",
        "GRAPH_RIGHT_PARENT",
    ));
    let mut p2_expected = marker_turn("GRAPH_USER_BASE_1", "GRAPH_BASE_1");
    p2_expected.extend(marker_turn("GRAPH_USER_BASE_2", "GRAPH_BASE_2"));
    p2_expected.extend(marker_turn("GRAPH_USER_RIGHT_PARENT", "GRAPH_RIGHT_PARENT"));
    let p2 = write_graph_node(
        codex_home,
        P2,
        "Mallard graph right parent",
        p2_records,
        p2_expected,
    );

    let mut p3_records = vec![session_meta_record(
        P3,
        project_root,
        Some(&p1_complete_cutoff),
    )];
    p3_records.extend(turn_records(
        P3,
        &fixture_uuid(0x131),
        &fixture_uuid(0x132),
        &fixture_uuid(0x133),
        Some(&fixture_uuid(0x113)),
        "GRAPH_USER_LEFT_LEAF",
        "GRAPH_LEFT_LEAF",
    ));
    let mut p3_expected = marker_turn("GRAPH_USER_BASE_1", "GRAPH_BASE_1");
    p3_expected.extend(marker_turn("GRAPH_USER_LEFT_PARENT", "GRAPH_LEFT_PARENT"));
    p3_expected.extend(marker_turn("GRAPH_USER_LEFT_LEAF", "GRAPH_LEFT_LEAF"));
    let p3 = write_graph_node(
        codex_home,
        P3,
        "Mallard graph left leaf",
        p3_records,
        p3_expected,
    );

    let p4_expected = marker_turn("GRAPH_USER_BASE_1", "GRAPH_BASE_1")
        .into_iter()
        .chain(marker_turn("GRAPH_USER_LEFT_PARENT", "GRAPH_LEFT_PARENT"))
        .collect();
    let p4 = write_graph_node(
        codex_home,
        P4,
        "Mallard graph inherited-only leaf",
        vec![session_meta_record(
            P4,
            project_root,
            Some(&p1_complete_cutoff),
        )],
        p4_expected,
    );

    nodes.insert("p0", p0);
    nodes.insert("p1", p1);
    nodes.insert("p2", p2);
    nodes.insert("p3", p3);
    nodes.insert("p4", p4);
    let graph = PaginatedGraph { nodes };
    for (name, segments) in [
        ("p0", vec![P0]),
        ("p1", vec![P0, P1]),
        ("p2", vec![P0, P2]),
        ("p3", vec![P0, P1, P3]),
        ("p4", vec![P0, P1, P4]),
    ] {
        validate_paginated_rollout(
            codex_home,
            graph.nodes.get(name).expect("graph node"),
            &segments,
            project_root,
        );
    }
    graph
}

#[allow(clippy::too_many_arguments)]
fn append_paginated_turn(
    codex_home: &Path,
    project_root: &Path,
    provider_thread_id: &str,
    turn_id: &str,
    user_item_id: &str,
    assistant_item_id: &str,
    user_marker: &str,
    assistant_marker: &str,
) {
    let path = rollout_path(codex_home, provider_thread_id);
    let lookup = |thread_id: &str| rollout_lookup(codex_home, thread_id);
    let before = read_bounded_file(
        &path,
        MAX_CONVERSATION_FILE_BYTES,
        "paginated rollout before append",
    );
    assert_eq!(before.last(), Some(&b'\n'));
    let resolved =
        resolve_codex_root(&path, &lookup).expect("cannot resolve paginated rollout before append");
    let parent = resolved
        .leaf_last_message_id
        .as_ref()
        .map(|message_id| message_id.as_str().to_string());
    let start_ordinal = before.iter().filter(|byte| **byte == b'\n').count() as u64;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open paginated rollout for append");
    let mut records = turn_records(
        provider_thread_id,
        turn_id,
        user_item_id,
        assistant_item_id,
        parent.as_deref(),
        user_marker,
        assistant_marker,
    );
    for (offset, record) in records.iter_mut().enumerate() {
        record
            .as_object_mut()
            .expect("appended rollout record must be an object")
            .insert("ordinal".to_string(), json!(start_ordinal + offset as u64));
        serde_json::to_writer(&mut file, record).expect("append paginated rollout record");
        file.write_all(b"\n")
            .expect("finish appended rollout record");
    }
    file.flush().expect("flush appended paginated rollout");
    let after = read_bounded_file(
        &path,
        MAX_CONVERSATION_FILE_BYTES,
        "paginated rollout after append",
    );
    assert!(
        after.starts_with(&before),
        "append changed the frozen prefix"
    );
    let refreshed = resolve_codex_root(&path, &lookup)
        .expect("production graph resolver rejected appended paginated rollout");
    let seeded = seed_codex_root(&refreshed, &after, None)
        .expect("production root seeding rejected appended paginated rollout");
    assert_eq!(
        seeded.mapping.consumed_record_ordinal,
        start_ordinal + records.len() as u64
    );
    build_standalone_codex_root(
        &refreshed,
        &StandaloneBuildMode::Encode {
            root: project_root.to_string_lossy().into_owned(),
            canonical_root: std::fs::canonicalize(project_root)
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
        },
        None,
    )
    .expect("standalone production builder rejected appended paginated rollout");
}

fn digest_inventory(codex_home: &Path) -> BTreeMap<PathBuf, [u8; 32]> {
    let mut inventory = BTreeMap::new();
    for root_name in ["sessions", "archived_sessions"] {
        let root = codex_home.join(root_name);
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(&root).follow_links(false).max_depth(8) {
            let entry = redacted(entry, "cannot inventory isolated Codex history");
            assert!(
                !entry.file_type().is_symlink(),
                "isolated Codex history contains a symlink"
            );
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(codex_home)
                .expect("history entry remains below Codex home")
                .to_path_buf();
            let bytes = read_bounded_file(
                entry.path(),
                MAX_CONVERSATION_FILE_BYTES,
                "isolated Codex history file",
            );
            inventory.insert(relative, Sha256::digest(bytes).into());
            assert!(
                inventory.len() <= MAX_SNAPSHOT_FILES,
                "isolated Codex history inventory exceeds its file bound"
            );
        }
    }
    let index = codex_home.join("session_index.jsonl");
    if index.exists() {
        inventory.insert(
            PathBuf::from("session_index.jsonl"),
            Sha256::digest(read_bounded_file(
                &index,
                MAX_CONVERSATION_FILE_BYTES,
                "isolated Codex session index",
            ))
            .into(),
        );
    }
    inventory
}

fn copy_snapshot_tree(source_home: &Path, snapshot_home: &Path, root_name: &str) {
    let source_root = source_home.join(root_name);
    if !source_root.exists() {
        return;
    }
    let mut entries_seen = 0_usize;
    for entry in WalkDir::new(&source_root).follow_links(false).max_depth(8) {
        let entry = redacted(entry, "cannot snapshot isolated Codex history");
        entries_seen += 1;
        assert!(
            entries_seen <= MAX_SNAPSHOT_FILES,
            "Codex snapshot exceeds its entry bound"
        );
        assert!(
            !entry.file_type().is_symlink(),
            "Codex snapshot source contains a symlink"
        );
        let relative = entry
            .path()
            .strip_prefix(source_home)
            .expect("snapshot source remains below Codex home");
        let destination = snapshot_home.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&destination).expect("create Codex snapshot directory");
        } else if entry.file_type().is_file() {
            let bytes = read_bounded_file(
                entry.path(),
                MAX_CONVERSATION_FILE_BYTES,
                "Codex snapshot source file",
            );
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).expect("create Codex snapshot file parent");
            }
            std::fs::write(&destination, bytes).expect("write Codex snapshot file");
        } else {
            panic!("Codex snapshot source contains a special file");
        }
    }
}

fn create_headless_snapshot(source_home: &Path, snapshot_home: &Path) {
    std::fs::create_dir_all(snapshot_home).expect("create headless Codex snapshot home");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(snapshot_home, std::fs::Permissions::from_mode(0o700))
            .expect("secure headless Codex snapshot home");
    }
    let auth = source_home.join("auth.json");
    let auth_bytes = read_bounded_file(&auth, MAX_AUTH_BYTES, "snapshot Codex authentication");
    let snapshot_auth = snapshot_home.join("auth.json");
    std::fs::write(&snapshot_auth, auth_bytes).expect("write snapshot Codex authentication");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&snapshot_auth, std::fs::Permissions::from_mode(0o600))
            .expect("secure snapshot Codex authentication");
    }
    copy_snapshot_tree(source_home, snapshot_home, "sessions");
    copy_snapshot_tree(source_home, snapshot_home, "archived_sessions");
    let index = source_home.join("session_index.jsonl");
    if index.exists() {
        let bytes = read_bounded_file(
            &index,
            MAX_CONVERSATION_FILE_BYTES,
            "snapshot Codex session index",
        );
        std::fs::write(snapshot_home.join("session_index.jsonl"), bytes)
            .expect("write snapshot Codex session index");
    }
}

fn history_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "history": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "role": {"enum": ["user", "assistant"]},
                        "marker": {"type": "string"},
                    },
                    "required": ["role", "marker"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["history"],
        "additionalProperties": false,
    })
}

fn run_headless_history_verification(
    request_count: &mut usize,
    source_codex_home: &Path,
    project_root: &Path,
    provider_thread_id: &str,
    expected: &[HistoryMarker],
) {
    assert!(
        provider_thread_id.len() <= 128
            && !provider_thread_id.is_empty()
            && !provider_thread_id.chars().any(char::is_control),
        "headless resume ID violates the test argument bound"
    );
    let original_before = digest_inventory(source_codex_home);
    let snapshot = tempfile::tempdir().expect("create disposable Codex verification snapshot");
    let snapshot_home = snapshot.path().join("codex-home");
    let snapshot_process_home = snapshot.path().join("process-home");
    std::fs::create_dir_all(&snapshot_process_home).expect("create snapshot process home");
    create_headless_snapshot(source_codex_home, &snapshot_home);
    let snapshot_rollout = rollout_path(&snapshot_home, provider_thread_id);
    let rollout_before = read_bounded_file(
        &snapshot_rollout,
        MAX_CONVERSATION_FILE_BYTES,
        "snapshot rollout before headless resume",
    );
    let schema_path = snapshot.path().join("history-schema.json");
    std::fs::write(
        &schema_path,
        serde_json::to_vec(&history_output_schema()).expect("serialize history output schema"),
    )
    .expect("write history output schema");
    let prompt = "Do not use tools. Inspect only history before this request. For each prior user message return its one token beginning LIVE_USER_ or GRAPH_USER_. For each prior assistant message return its entire marker beginning MALLARD_ or GRAPH_. Preserve message order and role. Ignore requested assistant markers quoted inside user messages. Do not include this request. Return only the schema object.";
    assert!(
        prompt.len() <= MAX_PROMPT_BYTES && !prompt.chars().any(char::is_control),
        "headless verification prompt violates the argument bound"
    );
    let arguments = vec![
        OsString::from("--ask-for-approval"),
        OsString::from("never"),
        OsString::from("exec"),
        OsString::from("--json"),
        OsString::from("--ignore-user-config"),
        OsString::from("--ignore-rules"),
        OsString::from("--skip-git-repo-check"),
        OsString::from("--sandbox"),
        OsString::from("read-only"),
        OsString::from("--model"),
        OsString::from(MODEL),
        OsString::from("--config"),
        OsString::from(REASONING_OVERRIDE),
        OsString::from("--cd"),
        project_root.as_os_str().to_owned(),
        OsString::from("resume"),
        OsString::from("--ephemeral"),
        OsString::from("--output-schema"),
        schema_path.as_os_str().to_owned(),
        OsString::from(provider_thread_id),
        OsString::from(prompt),
    ];
    assert!(
        arguments.len() <= MAX_COMMAND_ARGUMENTS
            && arguments
                .iter()
                .all(|argument| argument.as_encoded_bytes().len() <= MAX_COMMAND_ARGUMENT_BYTES),
        "headless Codex command violates the argv safety bound"
    );

    *request_count = request_count.saturating_add(1);
    let mut child = Command::new(codex_binary())
        .args(arguments)
        .current_dir(project_root)
        .env("CODEX_HOME", &snapshot_home)
        .env("HOME", &snapshot_process_home)
        .env("USERPROFILE", &snapshot_process_home)
        .env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_API_KEY")
        .env_remove("AZURE_OPENAI_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start headless Codex history verification");
    let stdout = child.stdout.take().expect("capture headless Codex stdout");
    let stderr = child.stderr.take().expect("capture headless Codex stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take((MAX_CODEX_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take((MAX_CODEX_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + CODEX_TIMEOUT;
    loop {
        if child
            .try_wait()
            .expect("wait for headless Codex verification")
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            panic!("headless Codex history verification exceeded its timeout");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let status = child.wait().expect("finish headless Codex verification");
    let stdout = stdout_reader
        .join()
        .expect("headless stdout reader panicked")
        .expect("collect headless Codex stdout");
    let stderr = stderr_reader
        .join()
        .expect("headless stderr reader panicked")
        .expect("collect headless Codex stderr");
    assert!(
        stdout.len() <= MAX_CODEX_OUTPUT_BYTES && stderr.len() <= MAX_CODEX_OUTPUT_BYTES,
        "headless Codex verification output exceeds its bound"
    );
    assert!(
        status.success(),
        "headless Codex verification failed without exposing provider output"
    );

    let stdout = String::from_utf8(stdout).expect("headless Codex JSON output must be UTF-8");
    let mut started_thread = None;
    let mut final_messages = Vec::new();
    for line in stdout.lines() {
        let event: Value =
            serde_json::from_str(line).expect("headless Codex emitted malformed JSONL");
        match event.get("type").and_then(Value::as_str) {
            Some("thread.started") => {
                started_thread = event
                    .get("thread_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            Some("item.completed") => {
                let item = event.get("item").expect("completed event lacks its item");
                match item.get("type").and_then(Value::as_str) {
                    Some("agent_message") => final_messages.push(
                        item.get("text")
                            .and_then(Value::as_str)
                            .expect("agent message lacks text")
                            .to_string(),
                    ),
                    Some("reasoning") => {}
                    Some(kind) => panic!("headless verification unexpectedly used item {kind}"),
                    None => panic!("headless verification item lacks its type"),
                }
            }
            Some("item.started") => {
                let kind = event
                    .get("item")
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                    .expect("started item lacks its type");
                assert!(
                    matches!(kind, "reasoning" | "agent_message"),
                    "headless history verification started forbidden item {kind}"
                );
            }
            _ => {}
        }
    }
    assert_eq!(started_thread.as_deref(), Some(provider_thread_id));
    assert_eq!(
        final_messages.len(),
        1,
        "headless verification did not return exactly one final response"
    );
    let report: HistoryReport = serde_json::from_str(&final_messages[0])
        .expect("headless history response violates its JSON schema");
    assert_eq!(
        report.history, expected,
        "Codex reconstructed old history incorrectly"
    );

    // Codex 0.147 appends to an existing rollout despite `--ephemeral`.
    // Always verify against a fresh disposable snapshot, prove that the
    // target was actually resumed there, and leave the real isolated home
    // byte-for-byte untouched.
    let rollout_after = read_bounded_file(
        &snapshot_rollout,
        MAX_CONVERSATION_FILE_BYTES,
        "snapshot rollout after headless resume",
    );
    assert!(
        rollout_after.len() > rollout_before.len() && rollout_after.starts_with(&rollout_before),
        "headless resume did not append its verification turn in the disposable snapshot"
    );
    assert_eq!(
        digest_inventory(source_codex_home),
        original_before,
        "headless verification mutated the original isolated Codex history"
    );
}

fn verify_all_histories(
    request_count: &mut usize,
    repositories: [(&LocalStateRepository, &LocalProjectId, &Path, &Path); 2],
    histories: &[(ThreadId, Vec<HistoryMarker>)],
) {
    for (repository, project_id, codex_home, project_root) in repositories {
        let before = digest_inventory(codex_home);
        for (logical_thread_id, expected) in histories {
            let provider_id = provider_thread_id(repository, project_id, logical_thread_id);
            run_headless_history_verification(
                request_count,
                codex_home,
                project_root,
                &provider_id,
                expected,
            );
        }
        assert_eq!(
            digest_inventory(codex_home),
            before,
            "complete headless verification matrix mutated an isolated Codex home"
        );
    }
}

fn local_storage(storage_root: &Path) -> StorageConfig {
    StorageConfig {
        id: StorageId::parse("storage-live-codex-e2e").expect("static storage id"),
        name: "Live Codex local storage".to_string(),
        kind: StorageKind::Local,
        bucket: String::new(),
        access_key_id: String::new(),
        secret_access_key: String::new(),
        account_id: String::new(),
        s3_endpoint: String::new(),
        region: String::new(),
        local_dir: storage_root.to_string_lossy().into_owned(),
        supports_conditional_writes: None,
    }
}

fn codex_key() -> ProviderKey {
    ProviderKey::parse("codex").expect("static provider key")
}

fn finish_setup(
    service: &ProjectSyncService<'_>,
    project_root: &Path,
    codex_home: &Path,
    storage: StorageConfig,
    repository_choice: DraftRepositoryChoice,
) -> ProjectDetail {
    let mut draft: ProjectSetupDraft = redacted(
        service.create_setup_draft(project_root),
        "cannot create project setup draft",
    )
    .draft;
    draft.profiles.insert(
        codex_key(),
        DraftProfileSelection::Pending {
            path: codex_home.to_string_lossy().into_owned(),
            display_name: String::new(),
        },
    );
    draft.storage = Some(DraftStorageSelection::Pending { storage });
    draft.repository = repository_choice;
    let draft = redacted(
        service.update_setup_draft(draft),
        "cannot save project setup choices",
    );
    let inspection = redacted(
        service.inspect_setup_draft(&draft.draft_id),
        "cannot inspect project setup",
    );
    assert!(inspection.can_finalize, "live project setup is not ready");
    redacted(
        service.finalize_project_setup(&draft.draft_id, draft.revision),
        "cannot finalize project setup",
    )
}

fn approved_pull_actions(plan: &RestorePlan) -> Vec<ActionId> {
    plan.actions
        .iter()
        .filter(|action| !matches!(action.kind, RestoreActionKind::Manual { .. }))
        .map(|action| action.action_id.clone())
        .chain(
            plan.conversation_actions
                .iter()
                .filter(|action| {
                    matches!(
                        action.action,
                        ConversationPlanActionType::AdoptCloudMetadata
                            | ConversationPlanActionType::MaterializeCloud
                            | ConversationPlanActionType::KeepBoth
                            | ConversationPlanActionType::CreateLocal
                    )
                })
                .map(|action| action.action_id.clone()),
        )
        .collect()
}

fn pull_and_apply(
    service: &ProjectSyncService<'_>,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    binding: &ProjectBinding,
) -> (RestorePlan, RestoreResult) {
    let plan = redacted(
        service.plan_bundle_restore(storage_id, bundle_id, binding),
        "cannot plan reviewed Pull",
    );
    let result = redacted(
        service.apply_bundle_restore(&plan.plan_id, approved_pull_actions(&plan)),
        "cannot apply reviewed Pull",
    );
    assert!(result.success, "reviewed Pull did not complete");
    assert!(
        result.failed_actions.is_empty(),
        "reviewed Pull had failures"
    );
    assert_eq!(
        result.reviewed_base_advanced,
        Some(true),
        "reviewed Pull did not advance its base"
    );
    (plan, result)
}

fn push(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> ProjectOperationResult {
    let review = redacted(
        service.prepare_push_review(project_id, storage_id),
        "cannot prepare reviewed Push",
    );
    assert!(
        matches!(
            review.destination_status,
            PushDestinationStatus::Ready { .. }
        ),
        "reviewed Push destination is not ready"
    );
    redacted(
        service.push_reviewed(
            project_id,
            storage_id,
            PushApproval::recommended(&review, storage_id),
        ),
        "cannot apply reviewed Push",
    )
}

fn provider_thread_id(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    logical_thread_id: &ThreadId,
) -> String {
    redacted(
        repository.load_conversation_bindings(),
        "cannot load conversation bindings",
    )
    .bindings
    .into_iter()
    .find(|binding| {
        binding.local_project_id == *project_id && binding.thread_id == *logical_thread_id
    })
    .and_then(|binding| binding.provider_thread_id)
    .expect("logical thread is not resumable")
}

fn conversation_binding(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    logical_thread_id: &ThreadId,
) -> LocalConversationBinding {
    redacted(
        repository.load_conversation_bindings(),
        "cannot load conversation bindings",
    )
    .bindings
    .into_iter()
    .find(|binding| {
        binding.local_project_id == *project_id && binding.thread_id == *logical_thread_id
    })
    .expect("logical thread has no local binding")
}

fn assert_rollout_cwd(codex_home: &Path, provider_thread_id: &str, expected_cwd: &Path) {
    let mut matches = Vec::new();
    for entry in WalkDir::new(codex_home.join("sessions")).follow_links(false) {
        let entry = redacted(entry, "cannot inspect isolated Codex sessions");
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
        {
            continue;
        }
        let bytes = read_bounded_file(
            entry.path(),
            MAX_CONVERSATION_FILE_BYTES,
            "isolated Codex rollout",
        );
        if let Ok(structure) = analyze_codex_rollout_structure(&bytes, Some(provider_thread_id)) {
            matches.push(structure);
        }
    }
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one rollout for the materialized Codex thread"
    );
    let expected = expected_cwd.to_string_lossy();
    assert!(
        matches[0].current.cwd == expected,
        "materialized Codex rollout has the wrong cwd"
    );
    assert!(
        matches[0]
            .structural_cwds
            .iter()
            .all(|cwd| cwd == expected.as_ref()),
        "materialized Codex rollout retained a foreign structural cwd"
    );
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn assert_bytes_contain_path(bytes: &[u8], path: &Path, label: &str) {
    let path = path
        .to_str()
        .expect("live path assertion requires a UTF-8 path");
    assert!(
        byte_slice_contains(bytes, path.as_bytes()),
        "{label} does not contain an expected path"
    );
}

fn assert_bytes_exclude_path(bytes: &[u8], path: &Path, label: &str) {
    let path = path
        .to_str()
        .expect("live path assertion requires a UTF-8 path");
    assert!(
        !byte_slice_contains(bytes, path.as_bytes()),
        "{label} retained a forbidden path"
    );
}

fn try_patch_apply_change_keys(bytes: &[u8]) -> Result<BTreeSet<String>, &'static str> {
    let mut patches = Vec::new();
    for raw in bytes
        .split(|byte| *byte == b'\n')
        .filter(|raw| !raw.is_empty())
    {
        let record: Value = serde_json::from_slice(raw).map_err(|_| "malformed_rollout_jsonl")?;
        let payload = record.get("payload").unwrap_or(&record);
        if payload.get("type").and_then(Value::as_str) != Some("patch_apply_end") {
            continue;
        }
        if payload.get("success").and_then(Value::as_bool) != Some(true) {
            return Err("patch_not_successful");
        }
        let changes = payload
            .get("changes")
            .and_then(Value::as_object)
            .ok_or("patch_changes_missing")?;
        patches.push(changes.keys().cloned().collect::<BTreeSet<_>>());
    }
    if patches.len() != 1 {
        return Err("patch_count_not_one");
    }
    patches.pop().ok_or("patch_count_not_one")
}

fn patch_apply_change_keys(bytes: &[u8]) -> BTreeSet<String> {
    try_patch_apply_change_keys(bytes)
        .unwrap_or_else(|reason| panic!("path-bearing rollout patch evidence failed: {reason}"))
}

fn local_patch_change_key_candidates(
    project_root: &Path,
    outside_root: &Path,
) -> Vec<BTreeSet<String>> {
    let canonical_project =
        std::fs::canonicalize(project_root).expect("canonicalize patch-key project root");
    let outside = canonical_fixture_paths(outside_root, &OUTSIDE_PROJECT_FIXTURES)
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    [project_root, canonical_project.as_path()]
        .into_iter()
        .map(|root| {
            IN_PROJECT_FIXTURES
                .iter()
                .map(|(relative, _)| {
                    root.join(clean_fixture_relative_path(relative))
                        .to_string_lossy()
                        .into_owned()
                })
                .chain(outside.iter().cloned())
                .collect::<BTreeSet<_>>()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn assert_local_patch_change_keys(bytes: &[u8], project_root: &Path, outside_root: &Path) {
    let actual = patch_apply_change_keys(bytes);
    assert!(
        local_patch_change_key_candidates(project_root, outside_root)
            .iter()
            .any(|expected| expected == &actual),
        "materialized rollout patch keys do not exactly match the local and outside fixtures"
    );
}

fn assert_portable_patch_change_keys(bytes: &[u8], outside_root: &Path) {
    let expected = IN_PROJECT_FIXTURES
        .iter()
        .map(|(relative, _)| format!("/__mallard_root__/{relative}"))
        .chain(
            canonical_fixture_paths(outside_root, &OUTSIDE_PROJECT_FIXTURES)
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned()),
        )
        .collect::<BTreeSet<_>>();
    assert!(
        patch_apply_change_keys(bytes) == expected,
        "portable rollout patch keys did not tokenize only in-project fixtures"
    );
}

fn assert_raw_rollout_paths(
    codex_home: &Path,
    provider_thread_id: &str,
    expected_project_root: &Path,
    outside_root: &Path,
    forbidden_project_root: Option<&Path>,
) {
    let bytes = read_bounded_file(
        &rollout_path(codex_home, provider_thread_id),
        MAX_CONVERSATION_FILE_BYTES,
        "live path rollout",
    );
    assert_local_patch_change_keys(&bytes, expected_project_root, outside_root);
    for (relative, _) in IN_PROJECT_FIXTURES {
        let spellings = fixture_path_spellings(expected_project_root, relative);
        assert!(
            spellings
                .iter()
                .any(|path| byte_slice_contains(&bytes, path.to_string_lossy().as_bytes())),
            "provider rollout does not contain an expected project-path spelling"
        );
    }
    for path in canonical_fixture_paths(outside_root, &OUTSIDE_PROJECT_FIXTURES) {
        assert_bytes_contain_path(&bytes, &path, "provider rollout");
    }
    if let Some(forbidden_project_root) = forbidden_project_root {
        for (relative, _) in IN_PROJECT_FIXTURES {
            for path in fixture_path_spellings(forbidden_project_root, relative) {
                assert_bytes_exclude_path(&bytes, &path, "remapped provider rollout");
            }
        }
    }
    assert!(
        !byte_slice_contains(&bytes, b"/__mallard_root__"),
        "materialized provider rollout retained the portable root token"
    );
    assert!(
        !byte_slice_contains(&bytes, b"__mallard_path_map__"),
        "materialized provider rollout retained the portable path-map marker"
    );
}

fn portable_conversation_rollout(
    service: &ProjectSyncService<'_>,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    storage_root: &Path,
    logical_thread_id: &ThreadId,
) -> Vec<u8> {
    let snapshot = redacted(
        service.inspect_bundle(storage_id, bundle_id),
        "cannot inspect the path-bearing bundle",
    );
    let scope = snapshot
        .manifest
        .scopes
        .get(&SyncScopeKey::provider(codex_key()))
        .expect("published bundle lacks its Codex scope");
    let conversation = scope
        .conversations
        .get(logical_thread_id)
        .expect("published bundle lacks its path-bearing conversation");
    let transcript = conversation
        .files
        .iter()
        .find(|file| file.role == ConversationFileRole::Transcript)
        .expect("path-bearing conversation lacks its transcript");
    let entry = scope
        .files
        .get(&transcript.logical_path)
        .expect("path-bearing transcript lacks a file entry");
    assert_eq!(
        entry.resource_id, conversation.resource_id,
        "path-bearing transcript belongs to another resource"
    );
    let key = repository_object_key(bundle_id, &entry.object_key)
        .expect("path-bearing transcript has an invalid object key");
    let bytes = read_bounded_file(
        &storage_root.join(key.as_str()),
        MAX_CONVERSATION_FILE_BYTES,
        "portable path-bearing rollout",
    );
    assert_eq!(bytes.len() as u64, entry.size);
    assert_eq!(hex_sha256(&bytes), entry.sha256);
    bytes
}

#[allow(clippy::too_many_arguments)]
fn assert_portable_rollout_paths(
    service: &ProjectSyncService<'_>,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    storage_root: &Path,
    logical_thread_id: &ThreadId,
    source_project_root: &Path,
    target_project_root: &Path,
    outside_root: &Path,
) {
    let bytes = portable_conversation_rollout(
        service,
        storage_id,
        bundle_id,
        storage_root,
        logical_thread_id,
    );
    assert_portable_patch_change_keys(&bytes, outside_root);
    for (relative, _) in IN_PROJECT_FIXTURES {
        let token_path = format!("/__mallard_root__/{relative}");
        assert!(
            byte_slice_contains(&bytes, token_path.as_bytes()),
            "portable rollout lacks an expected tokenized project path"
        );
    }
    for path in canonical_fixture_paths(outside_root, &OUTSIDE_PROJECT_FIXTURES) {
        assert_bytes_contain_path(&bytes, &path, "portable rollout");
    }
    for root in [source_project_root, target_project_root] {
        for (relative, _) in IN_PROJECT_FIXTURES {
            for path in fixture_path_spellings(root, relative) {
                assert_bytes_exclude_path(&bytes, &path, "portable rollout");
            }
        }
    }
}

fn assert_clean_affected_path(path: &str) {
    let path = Path::new(path);
    assert!(
        !path.as_os_str().is_empty()
            && !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "Session Details exposed an unsafe affected path"
    );
}

#[allow(clippy::too_many_arguments)]
fn assert_session_detail_paths(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    resource_id: &ResourceId,
    project_root: &Path,
    codex_home: &Path,
    provider_thread_id: &str,
    outside_root: &Path,
) {
    let header = service
        .get_project_session_detail(project_id, resource_id)
        .expect("cannot load path-bearing Session Details header");
    assert_eq!(header.resource_id, resource_id.as_str());
    assert_eq!(header.provider, codex_key());
    assert_eq!(header.provider_session_id, provider_thread_id);
    assert!(header.capabilities.tool_calls);
    assert!(header.capabilities.event_detail);

    let included_event_kinds = BTreeSet::from([SessionTimelineEventKind::ToolCall]);
    let included_tool_categories = BTreeSet::from([
        SessionToolCategory::FileEdit,
        SessionToolCategory::Shell,
        SessionToolCategory::Read,
        SessionToolCategory::Other,
    ]);
    let required_paths = IN_PROJECT_FIXTURES
        .iter()
        .map(|(relative, _)| relative.to_string())
        .collect::<BTreeSet<_>>();
    let external_paths = canonical_fixture_paths(outside_root, &OUTSIDE_PROJECT_FIXTURES)
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let mut visible = serde_json::to_string(&header).expect("serialize Session Details header");
    let mut cursor = None;
    let mut seen_cursors = BTreeSet::new();
    let mut seen_event_ids = BTreeSet::new();
    let mut path_event_count = 0_usize;
    let mut event_count = 0_usize;

    loop {
        let page = service
            .get_project_session_timeline_page(
                project_id,
                resource_id,
                ProjectSessionTimelineRequest {
                    snapshot_id: header.snapshot_id.clone(),
                    cursor: cursor.clone(),
                    anchor_event_id: None,
                    limit: Some(1),
                    included_event_kinds: included_event_kinds.clone(),
                    included_tool_categories: included_tool_categories.clone(),
                    turn_id: None,
                },
            )
            .expect("cannot load a path-bearing Session Details timeline page");
        assert_eq!(page.resource_id, resource_id.as_str());
        assert_eq!(page.snapshot_id, header.snapshot_id);
        assert!(
            page.events.len() <= 1,
            "Session Details ignored limit-one pagination"
        );
        for event in page.events {
            event_count = event_count.saturating_add(1);
            assert!(
                event_count <= MAX_SESSION_DETAIL_EVENTS,
                "Session Details timeline exceeds the live-test event bound"
            );
            assert_eq!(event.kind, SessionTimelineEventKind::ToolCall);
            assert!(
                seen_event_ids.insert(event.event_id.clone()),
                "Session Details pagination repeated an event"
            );
            let tool = event
                .tool
                .as_ref()
                .expect("tool timeline event lacks its preview");
            for path in &tool.affected_paths {
                assert_clean_affected_path(path);
                assert!(
                    !external_paths.iter().any(|external| external == path),
                    "outside path entered the Session Details Paths list"
                );
            }
            let affected = tool.affected_paths.iter().cloned().collect::<BTreeSet<_>>();
            if required_paths.is_subset(&affected) {
                path_event_count = path_event_count.saturating_add(1);
            }
            if let Some(arguments) = &tool.argument_preview {
                visible.push('\n');
                visible.push_str(arguments);
            }
            if let Some(result) = &tool.result_preview {
                visible.push('\n');
                visible.push_str(result);
            }

            let detail = service
                .get_project_session_event_detail(
                    project_id,
                    resource_id,
                    &header.snapshot_id,
                    &event.event_id,
                )
                .expect("cannot expand a path-bearing Session Details event");
            assert_eq!(detail.event_id, event.event_id);
            assert_eq!(detail.affected_paths, tool.affected_paths);
            for path in &detail.affected_paths {
                assert_clean_affected_path(path);
            }
            if let Some(arguments) = &detail.arguments {
                visible.push('\n');
                visible.push_str(arguments);
            }
            if let Some(result) = &detail.result {
                visible.push('\n');
                visible.push_str(result);
            }
        }

        let Some(next_cursor) = page.next_cursor else {
            break;
        };
        assert!(
            seen_cursors.insert(next_cursor.clone()),
            "Session Details pagination repeated a cursor"
        );
        assert!(
            seen_cursors.len() <= MAX_SESSION_DETAIL_EVENTS,
            "Session Details pagination exceeds the live-test page bound"
        );
        cursor = Some(next_cursor);
    }

    assert!(
        event_count > 0,
        "Session Details did not expose tool events"
    );
    assert_eq!(
        header.event_counts.tool_calls.value,
        Some(event_count as u64),
        "Session Details pagination did not visit every counted tool event"
    );
    assert_eq!(
        path_event_count, 1,
        "Session Details did not expose exactly one file operation with both project-relative paths"
    );
    for (relative, _) in IN_PROJECT_FIXTURES {
        assert!(
            visible.contains(&format!("./{relative}")),
            "Session Details did not display a normalized in-project path"
        );
    }
    for external in &external_paths {
        assert!(
            visible.contains(external),
            "Session Details did not display an unchanged outside path"
        );
    }
    let transcript = rollout_path(codex_home, provider_thread_id);
    let canonical_project =
        std::fs::canonicalize(project_root).expect("canonicalize Session Details project root");
    let canonical_home =
        std::fs::canonicalize(codex_home).expect("canonicalize Session Details provider home");
    let mut privacy_view = visible.clone();
    for external in &external_paths {
        privacy_view = privacy_view.replace(external, "<expected-outside-path>");
    }
    for forbidden in [
        project_root.to_string_lossy(),
        canonical_project.to_string_lossy(),
        codex_home.to_string_lossy(),
        canonical_home.to_string_lossy(),
        transcript.to_string_lossy(),
    ] {
        assert!(
            !privacy_view.contains(forbidden.as_ref()),
            "Session Details leaked a protected path outside the expected outside fixtures"
        );
    }
    assert!(
        !visible.contains("/__mallard_root__"),
        "Session Details exposed the portable root token"
    );
    assert!(
        !visible.contains("__mallard_path_map__"),
        "Session Details exposed the portable path-map marker"
    );
}

fn logical_thread_id(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    provider_thread_id: &str,
) -> ThreadId {
    redacted(
        repository.load_conversation_bindings(),
        "cannot load conversation bindings",
    )
    .bindings
    .into_iter()
    .find(|binding| {
        binding.local_project_id == *project_id
            && binding.provider_thread_id.as_deref() == Some(provider_thread_id)
    })
    .map(|binding| binding.thread_id)
    .expect("Mallard did not bind the real Codex thread")
}

fn keep_both_for(plan: &RestorePlan, logical_thread_id: &ThreadId) -> KeepBothPlan {
    plan.conversation_actions
        .iter()
        .find(|action| {
            action.thread_id == *logical_thread_id
                && action.action == ConversationPlanActionType::KeepBoth
        })
        .and_then(|action| action.keep_both.clone())
        .expect("divergent Pull does not offer Keep Both")
}

fn assert_keep_both_result(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    logical_thread_id: &ThreadId,
    old_provider_thread_id: &str,
    keep_both: &KeepBothPlan,
    result: &RestoreResult,
) {
    assert_eq!(keep_both.preserved.preserved_from, *logical_thread_id);
    assert!(keep_both.preserved.ordinal > 0);
    assert!(keep_both.preserved_display_title.contains("Preserved "));
    assert_eq!(result.preserved_threads.len(), 1);
    let preserved = &result.preserved_threads[0];
    assert_eq!(preserved.original_thread_id, *logical_thread_id);
    assert_eq!(preserved.preserved_thread_id, keep_both.preserved_thread_id);
    assert_eq!(
        preserved.preserved_display_title,
        keep_both.preserved_display_title
    );
    assert_eq!(
        provider_thread_id(repository, project_id, logical_thread_id),
        keep_both.remote_provider_thread_id
    );
    assert_ne!(
        keep_both.remote_provider_thread_id, old_provider_thread_id,
        "Keep Both reused the divergent provider thread for the cloud side"
    );
    assert_eq!(
        provider_thread_id(repository, project_id, &keep_both.preserved_thread_id),
        old_provider_thread_id,
        "Keep Both did not retain the local provider thread on the preserved logical ID"
    );
    assert_eq!(
        conversation_binding(repository, project_id, &keep_both.preserved_thread_id).preserved,
        Some(keep_both.preserved.clone()),
        "Keep Both did not persist preservation provenance"
    );
}

fn assert_divergent_push_gate(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    storage_id: &StorageId,
    logical_thread_id: &ThreadId,
    replica: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let comparison = redacted(
            service.compare_threads(project_id, storage_id),
            "cannot refresh the divergent thread comparison",
        );
        if comparison.entries.iter().any(|entry| {
            entry.cloud_thread_id.as_ref() == Some(logical_thread_id)
                && entry.state == ThreadSyncState::Diverged
        }) {
            break;
        }
        if Instant::now() >= deadline {
            panic!(
                "{replica} did not observe the divergent Codex update before the watcher deadline"
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let review = redacted(
        service.prepare_push_review(project_id, storage_id),
        "cannot prepare divergent Push review",
    );
    assert!(matches!(
        review.destination_status,
        PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::RemoteAdvanced,
            ..
        }
    ));
    let comparison = review
        .thread_comparison
        .as_ref()
        .expect("divergent Push review lacks thread comparison");
    let entry = comparison
        .entries
        .iter()
        .find(|entry| {
            entry.cloud_thread_id.as_ref() == Some(logical_thread_id)
                && entry.state == ThreadSyncState::Diverged
        })
        .unwrap_or_else(|| {
            panic!(
                "{replica} Push review lacks the expected divergent logical conversation (diverged={}, rows={})",
                comparison.counts.diverged,
                comparison.entries.len()
            )
        });
    assert_eq!(
        entry.push_decision.action,
        ConversationReviewAction::RequirePull
    );
    assert!(!entry.push_decision.selectable);
    assert!(
        service
            .push_reviewed(
                project_id,
                storage_id,
                PushApproval::recommended(&review, storage_id),
            )
            .is_err(),
        "a stale replica unexpectedly pushed before Pull"
    );
}

fn wait_for_thread_states(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    storage_id: &StorageId,
    expected: &[(&ThreadId, ThreadSyncState)],
    replica: &str,
) -> ThreadSyncComparison {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let comparison = redacted(
            service.compare_threads(project_id, storage_id),
            "cannot refresh paginated thread comparison",
        );
        let matches = expected.iter().all(|(thread_id, state)| {
            comparison.entries.iter().any(|entry| {
                entry.cloud_thread_id.as_ref() == Some(*thread_id) && entry.state == *state
            })
        });
        if matches {
            return comparison;
        }
        if Instant::now() >= deadline {
            let observed = comparison
                .entries
                .iter()
                .filter_map(|entry| {
                    entry
                        .cloud_thread_id
                        .as_ref()
                        .map(|thread_id| format!("{thread_id}: {:?}", entry.state))
                })
                .collect::<Vec<_>>()
                .join(", ");
            panic!("{replica} did not observe expected paginated states; observed {observed}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn assert_stale_push_rejected(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    storage_id: &StorageId,
    review: &tauri_app_lib::PushReviewContext,
) {
    assert!(
        service
            .push_reviewed(
                project_id,
                storage_id,
                PushApproval::recommended(review, storage_id),
            )
            .is_err(),
        "reviewed Push with a stale storage-head pin unexpectedly succeeded"
    );
}

fn assert_pulled_standalone_rollout(
    codex_home: &Path,
    project_root: &Path,
    provider_thread_id: &str,
    expected: &[HistoryMarker],
) {
    let path = rollout_path(codex_home, provider_thread_id);
    let bytes = read_bounded_file(
        &path,
        MAX_CONVERSATION_FILE_BYTES,
        "pulled standalone paginated rollout",
    );
    let records = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<Value>(line).expect("parse pulled rollout record"))
        .collect::<Vec<_>>();
    assert!(!records.is_empty());
    assert_eq!(records[0]["payload"]["history_mode"], json!("paginated"));
    assert!(records[0]["payload"]["history_base"].is_null());
    assert_eq!(
        records
            .iter()
            .map(|record| record["ordinal"].as_u64().expect("pulled ordinal"))
            .collect::<Vec<_>>(),
        (0..records.len() as u64).collect::<Vec<_>>()
    );
    assert_rollout_cwd(codex_home, provider_thread_id, project_root);
    let lookup = |thread_id: &str| rollout_lookup(codex_home, thread_id);
    let resolved = resolve_codex_root(&path, &lookup)
        .expect("production graph resolver rejected pulled standalone rollout");
    assert_eq!(resolved.segments.len(), 1);
    assert_eq!(resolved.chain.len(), expected.len());
    let artifact = build_standalone_codex_root(
        &resolved,
        &StandaloneBuildMode::Encode {
            root: project_root.to_string_lossy().into_owned(),
            canonical_root: std::fs::canonicalize(project_root)
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
        },
        None,
    )
    .expect("production standalone builder rejected pulled rollout");
    assert_eq!(artifact.message_count as usize, expected.len());
}

fn byte_slice_contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn collect_json_string_needles(value: &serde_json::Value, needles: &mut Vec<Vec<u8>>) {
    match value {
        serde_json::Value::String(value) if value.len() >= 16 => {
            needles.push(value.as_bytes().to_vec());
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_json_string_needles(value, needles);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                collect_json_string_needles(value, needles);
            }
        }
        _ => {}
    }
}

fn credential_needles(auth_path: &Path) -> Vec<Vec<u8>> {
    let auth = read_bounded_file(
        auth_path,
        MAX_AUTH_BYTES,
        "copied Codex authentication file",
    );
    let mut needles = Vec::new();
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&auth) {
        collect_json_string_needles(&value, &mut needles);
    }
    let digest = Sha256::digest(auth);
    needles.push(digest.to_vec());
    needles.push(
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
            .into_bytes(),
    );
    needles
}

fn assert_no_sensitive_bytes(haystack: &[u8], sensitive: &[Vec<u8>], label: &str) {
    assert!(
        sensitive
            .iter()
            .all(|needle| !byte_slice_contains(haystack, needle)),
        "{label} contains isolated credential or path material"
    );
}

fn assert_portable_state_is_safe(
    service: &ProjectSyncService<'_>,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    storage_root: &Path,
    credential_needles: &[Vec<u8>],
    path_needles: &[Vec<u8>],
) {
    let snapshot = redacted(
        service.inspect_bundle(storage_id, bundle_id),
        "cannot inspect published bundle",
    );
    let manifest = serde_json::to_vec(&snapshot.manifest).expect("serialize bundle manifest");
    assert!(
        !byte_slice_contains(&manifest, b"auth.json"),
        "portable manifest names the authentication file"
    );
    assert_no_sensitive_bytes(&manifest, credential_needles, "portable manifest");
    assert_no_sensitive_bytes(&manifest, path_needles, "portable manifest");

    for entry in WalkDir::new(storage_root).follow_links(false) {
        let entry = redacted(entry, "cannot inspect isolated local storage");
        let metadata = redacted(entry.metadata(), "cannot inspect isolated storage entry");
        assert!(
            !metadata.file_type().is_symlink(),
            "storage contains a symlink"
        );
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        assert!(
            name != "auth.json",
            "storage contains an authentication file"
        );
        let object_key = entry
            .path()
            .strip_prefix(storage_root)
            .expect("storage entry remains below its root")
            .to_string_lossy();
        assert_no_sensitive_bytes(
            object_key.as_bytes(),
            credential_needles,
            "storage object key",
        );
        assert_no_sensitive_bytes(object_key.as_bytes(), path_needles, "storage object key");
        if metadata.is_file() {
            let bytes = read_bounded_file(
                entry.path(),
                MAX_STORAGE_OBJECT_BYTES,
                "isolated storage object",
            );
            assert!(
                !byte_slice_contains(&bytes, b"auth.json"),
                "storage object contains an authentication filename"
            );
            assert_no_sensitive_bytes(&bytes, credential_needles, "storage object");
        }
    }
}

fn storage_digest_inventory(storage_root: &Path) -> BTreeMap<PathBuf, [u8; 32]> {
    let mut inventory = BTreeMap::new();
    for entry in WalkDir::new(storage_root).follow_links(false).max_depth(16) {
        let entry = redacted(entry, "cannot inventory isolated local storage");
        assert!(
            !entry.file_type().is_symlink(),
            "isolated local storage contains a symlink"
        );
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(storage_root)
            .expect("storage object remains below its root")
            .to_path_buf();
        let bytes = read_bounded_file(
            entry.path(),
            MAX_STORAGE_OBJECT_BYTES,
            "isolated storage object",
        );
        inventory.insert(relative, Sha256::digest(bytes).into());
        assert!(
            inventory.len() <= 50_000,
            "isolated storage inventory exceeds its file bound"
        );
    }
    inventory
}

fn assert_fixed_point(comparison: &ThreadSyncComparison, revision: u64) {
    assert_eq!(comparison.revision, Some(revision));
    assert_eq!(comparison.base_revision, Some(revision));
    assert!(
        comparison
            .entries
            .iter()
            .all(|entry| entry.state == ThreadSyncState::Synced),
        "fixed-point comparison still has applicable thread work"
    );
    assert_eq!(comparison.counts.diverged, 0);
    assert_eq!(comparison.counts.local, 0);
    assert_eq!(comparison.counts.storage, 0);
}

fn assert_pull_is_no_op(
    service: &ProjectSyncService<'_>,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    binding: &ProjectBinding,
) {
    let plan = redacted(
        service.plan_bundle_restore(storage_id, bundle_id, binding),
        "cannot plan fixed-point Pull",
    );
    assert!(
        approved_pull_actions(&plan).is_empty(),
        "fixed-point Pull still has applicable actions"
    );
}

fn assert_offline_path_timeline(
    rollout: &Path,
    project_root: &Path,
    expected_argument_root: &Path,
    outside_root: &Path,
    expected_provider_call_id: &str,
) {
    let adapter = ProviderRegistry::production()
        .require_sessions(&codex_key())
        .expect("Codex session adapter is unavailable");
    let context = SessionTimelineContext {
        resource_id: "codex:conversation:offline-path-fixture",
        provider_session_id: "offline-path-fixture",
        project_root,
        project_root_aliases: &[],
    };
    let index = adapter
        .build_timeline_index(rollout, &context)
        .expect("current Codex path fixture did not index");
    let tool_entry = index
        .events
        .iter()
        .find(|event| event.kind == SessionTimelineEventKind::ToolCall)
        .expect("current Codex path fixture lacks its durable tool event");
    let tool = tool_entry
        .tool
        .as_ref()
        .expect("current Codex path fixture tool lacks its index entry");
    assert_eq!(
        tool.provider_call_id, expected_provider_call_id,
        "current Codex patch paths attached to the wrong durable tool"
    );
    assert_eq!(
        tool.affected_paths.iter().cloned().collect::<BTreeSet<_>>(),
        IN_PROJECT_FIXTURES
            .iter()
            .map(|(relative, _)| relative.to_string())
            .collect::<BTreeSet<_>>()
    );
    assert!(tool.affected_paths.iter().all(|path| {
        let path = Path::new(path);
        !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
    }));
    let page = adapter
        .read_timeline_page(
            rollout,
            &index,
            &SessionTimelineQuery {
                start_index: None,
                anchor_event_id: None,
                limit: 1,
                included_event_kinds: BTreeSet::from([SessionTimelineEventKind::ToolCall]),
                included_tool_categories: BTreeSet::from([
                    SessionToolCategory::FileEdit,
                    SessionToolCategory::Shell,
                    SessionToolCategory::Read,
                    SessionToolCategory::Other,
                ]),
                turn_id: None,
            },
        )
        .expect("current Codex path fixture timeline did not load");
    assert_eq!(page.events.len(), 1);
    let detail = adapter
        .read_event_detail(rollout, &index, &tool_entry.event_id)
        .expect("current Codex path fixture event did not expand");
    assert_eq!(detail.affected_paths, tool.affected_paths);
    let arguments = detail
        .arguments
        .expect("current Codex path fixture lacks tool arguments");
    for path in canonical_fixture_paths(expected_argument_root, &IN_PROJECT_FIXTURES) {
        assert!(arguments.contains(path.to_string_lossy().as_ref()));
    }
    for path in canonical_fixture_paths(outside_root, &OUTSIDE_PROJECT_FIXTURES) {
        assert!(arguments.contains(path.to_string_lossy().as_ref()));
        assert!(!tool
            .affected_paths
            .iter()
            .any(|affected| affected == path.to_string_lossy().as_ref()));
    }
}

#[test]
fn revision_one_prompt_and_diagnostics_preserve_only_safe_evidence() {
    let selected_project = Path::new("/var/folders/mallard-live/project1");
    let canonical_external = Path::new("/private/var/folders/mallard-live/project1_external");
    let prompt = revision_one_file_prompt(selected_project, canonical_external);
    assert_eq!(prompt.matches("await tools.apply_patch(...)").count(), 1);
    for (relative, _) in IN_PROJECT_FIXTURES {
        assert!(prompt.contains(selected_project.join(relative).to_string_lossy().as_ref()));
        assert!(!prompt.contains(
            Path::new("/private/var/folders/mallard-live/project1")
                .join(relative)
                .to_string_lossy()
                .as_ref()
        ));
    }
    for (relative, _) in OUTSIDE_PROJECT_FIXTURES {
        assert!(prompt.contains(canonical_external.join(relative).to_string_lossy().as_ref()));
    }

    let stdout = br#"{"type":"thread.started","thread_id":"safe-thread"}
{"type":"item.completed","item":{"type":"agent_message","text":"MALLARD_BASE /private/secret"}}
"#;
    let events = parse_codex_output_events(stdout).expect("parse diagnostic fixture JSONL");
    let event_synopsis = codex_event_synopsis(&events);
    assert!(event_synopsis.contains("type=thread.started"));
    assert!(event_synopsis.contains("item=agent_message"));
    assert!(!event_synopsis.contains("MALLARD_BASE"));
    assert!(!event_synopsis.contains("/private/secret"));
    let credential_shaped = parse_codex_output_events(
        br#"{"type":"error","error":{"code":"sk-proj-private-token"}}
"#,
    )
    .expect("parse credential-shaped diagnostic fixture");
    let credential_synopsis = codex_event_synopsis(&credential_shaped);
    assert!(credential_synopsis.contains("type=error"));
    assert!(credential_synopsis.contains("code=other"));
    assert!(!credential_synopsis.contains("sk-proj-private-token"));

    let stderr_synopsis =
        codex_stderr_synopsis(b"error: sandbox permission denied at /private/secret");
    assert!(stderr_synopsis.contains("sandbox=true"));
    assert!(stderr_synopsis.contains("permission_denied=true"));
    assert!(!stderr_synopsis.contains("/private/secret"));
    let malformed = parse_codex_output_events(b"{\"message\":\"/private/secret\"\n")
        .expect_err("malformed diagnostic JSONL was accepted");
    assert_eq!(malformed, "malformed_stdout_jsonl");
    assert!(!malformed.contains("/private/secret"));
    let overlong = "{}\n".repeat(MAX_CODEX_OUTPUT_EVENTS + 1);
    assert_eq!(
        parse_codex_output_events(overlong.as_bytes()),
        Err("stdout_event_limit")
    );
}

#[test]
fn current_codex_nested_file_edit_paths_survive_wire_projection() {
    let root = tempfile::tempdir().expect("create current Codex path fixture root");
    let project1 = root.path().join("project1");
    let project1_copy = root.path().join("project1_copy");
    let project1_external = root.path().join("project1_external");
    for directory in [&project1, &project1_copy, &project1_external] {
        std::fs::create_dir_all(directory).expect("create current Codex path fixture directory");
    }
    let project1 = std::fs::canonicalize(project1).expect("canonicalize source fixture project");
    let project1_copy =
        std::fs::canonicalize(project1_copy).expect("canonicalize target fixture project");
    let project1_external =
        std::fs::canonicalize(project1_external).expect("canonicalize outside fixture root");
    let source_thread_id = fixture_uuid(0x401);
    let target_thread_id = fixture_uuid(0x402);
    let inside_paths = canonical_fixture_paths(&project1, &IN_PROJECT_FIXTURES);
    let outside_paths = canonical_fixture_paths(&project1_external, &OUTSIDE_PROJECT_FIXTURES);
    let input = inside_paths
        .iter()
        .chain(outside_paths.iter())
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let changes = json!({
        (inside_paths[0].to_string_lossy().into_owned()): {"type": "add"},
        (inside_paths[1].to_string_lossy().into_owned()): {"type": "add"},
        (outside_paths[0].to_string_lossy().into_owned()): {"type": "add"},
        (outside_paths[1].to_string_lossy().into_owned()): {"type": "add"},
    });
    let user_message_id = fixture_uuid(0x404);
    let assistant_message_id = fixture_uuid(0x405);
    let tool_message_id = fixture_uuid(0x406);
    let tool_output_message_id = fixture_uuid(0x407);
    let turn = [
        json!({
            "timestamp": "2026-08-18T00:00:01Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": user_message_id,
                "parent_id": null,
                "role": "user",
                "content": [{"type": "input_text", "text": "LIVE_USER_R1"}],
            },
        }),
        json!({
            "timestamp": "2026-08-18T00:00:02Z",
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call",
                "id": tool_message_id,
                "parent_id": user_message_id,
                "call_id": "exec-path-fixture",
                "name": "exec",
                "input": input,
            },
        }),
        json!({
            "timestamp": "2026-08-18T00:00:02Z",
            "type": "event_msg",
            "payload": {
                "type": "patch_apply_end",
                "call_id": "nested-patch-path-fixture",
                "success": true,
                "status": "completed",
                "changes": changes.clone(),
            },
        }),
        json!({
            "timestamp": "2026-08-18T00:00:02Z",
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call_output",
                "id": tool_output_message_id,
                "parent_id": tool_message_id,
                "call_id": "exec-path-fixture",
                "output": "done",
            },
        }),
        json!({
            "timestamp": "2026-08-18T00:00:03Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": assistant_message_id,
                "parent_id": tool_output_message_id,
                "role": "assistant",
                "content": [{"type": "output_text", "text": "MALLARD_BASE"}],
                "phase": "final_answer",
            },
        }),
    ];
    let mut records = vec![session_meta_record(&source_thread_id, &project1, None)];
    records.extend(turn);
    let source_bytes = encode_rollout_records(records);
    assert_local_patch_change_keys(&source_bytes, &project1, &project1_external);
    let source_rollout = root.path().join("source.jsonl");
    std::fs::write(&source_rollout, &source_bytes).expect("write source path fixture rollout");
    assert_offline_path_timeline(
        &source_rollout,
        &project1,
        &project1,
        &project1_external,
        "exec-path-fixture",
    );

    let no_bases = |_: &str| Ok(None);
    let source = resolve_codex_root(&source_rollout, &no_bases)
        .expect("resolve source current Codex path fixture");
    let wire = build_standalone_codex_root(
        &source,
        &StandaloneBuildMode::Encode {
            root: project1.to_string_lossy().into_owned(),
            canonical_root: Some(project1.to_string_lossy().into_owned()),
        },
        None,
    )
    .expect("encode current Codex path fixture");
    assert_portable_patch_change_keys(&wire.root_bytes, &project1_external);
    for (relative, _) in IN_PROJECT_FIXTURES {
        assert!(byte_slice_contains(
            &wire.root_bytes,
            format!("/__mallard_root__/{relative}").as_bytes()
        ));
    }
    for path in &inside_paths {
        assert_bytes_exclude_path(&wire.root_bytes, path, "offline portable rollout");
    }
    for path in &outside_paths {
        assert_bytes_contain_path(&wire.root_bytes, path, "offline portable rollout");
    }

    let wire_rollout = root.path().join("wire.jsonl");
    std::fs::write(&wire_rollout, &wire.root_bytes).expect("write portable path fixture rollout");
    let wire_root = resolve_codex_root(&wire_rollout, &no_bases)
        .expect("resolve portable current Codex path fixture");
    let target = build_standalone_codex_root(
        &wire_root,
        &StandaloneBuildMode::Decode {
            root_provider_thread_id: target_thread_id,
            target_root: project1_copy.to_string_lossy().into_owned(),
            canonical_target_root: Some(project1_copy.to_string_lossy().into_owned()),
        },
        None,
    )
    .expect("decode current Codex path fixture");
    assert_local_patch_change_keys(&target.root_bytes, &project1_copy, &project1_external);
    let target_rollout = root.path().join("target.jsonl");
    std::fs::write(&target_rollout, &target.root_bytes).expect("write target path fixture rollout");
    for path in canonical_fixture_paths(&project1_copy, &IN_PROJECT_FIXTURES) {
        assert_bytes_contain_path(&target.root_bytes, &path, "offline target rollout");
    }
    for path in &inside_paths {
        assert_bytes_exclude_path(&target.root_bytes, path, "offline target rollout");
    }
    for path in &outside_paths {
        assert_bytes_contain_path(&target.root_bytes, path, "offline target rollout");
    }
    assert!(!byte_slice_contains(
        &target.root_bytes,
        b"/__mallard_root__"
    ));
    assert!(!byte_slice_contains(
        &target.root_bytes,
        b"__mallard_path_map__"
    ));
    assert_offline_path_timeline(
        &target_rollout,
        &project1_copy,
        &project1_copy,
        &project1_external,
        "nested-patch-path-fixture",
    );
}

#[test]
fn current_codex_paginated_graph_fixture_passes_production_validators() {
    let root = tempfile::tempdir().expect("create paginated fixture test root");
    let codex_home = root.path().join("codex-home");
    let project_root = root.path().join("project");
    std::fs::create_dir_all(&codex_home).expect("create fixture Codex home");
    std::fs::create_dir_all(&project_root).expect("create fixture project");
    let graph = write_paginated_graph(&codex_home, &project_root);
    assert_eq!(graph.nodes.len(), 5);
    assert_eq!(graph.nodes["p0"].expected.len(), 4);
    assert_eq!(graph.nodes["p1"].expected.len(), 4);
    assert_eq!(graph.nodes["p2"].expected.len(), 6);
    assert_eq!(graph.nodes["p3"].expected.len(), 6);
    assert_eq!(graph.nodes["p4"].expected.len(), 4);

    // A remote update is materialized as a standalone paginated rollout. It
    // intentionally retains the resolved history while no longer depending
    // on the source machine's native base file.
    let p2_path = graph.nodes["p2"].path.clone();
    let lookup = |thread_id: &str| rollout_lookup(&codex_home, thread_id);
    let resolved = resolve_codex_root(&p2_path, &lookup).expect("resolve native P2 fixture");
    let wire = build_standalone_codex_root(
        &resolved,
        &StandaloneBuildMode::Encode {
            root: project_root.to_string_lossy().into_owned(),
            canonical_root: std::fs::canonicalize(&project_root)
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
        },
        None,
    )
    .expect("encode standalone P2 fixture");
    std::fs::write(&p2_path, wire.root_bytes).expect("stage wire P2 fixture");
    let wire_resolved = resolve_codex_root(&p2_path, &lookup).expect("resolve wire P2 fixture");
    let local = build_standalone_codex_root(
        &wire_resolved,
        &StandaloneBuildMode::Decode {
            root_provider_thread_id: P2.to_string(),
            target_root: project_root.to_string_lossy().into_owned(),
            canonical_target_root: std::fs::canonicalize(&project_root)
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
        },
        None,
    )
    .expect("decode standalone P2 fixture");
    std::fs::write(&p2_path, local.root_bytes).expect("materialize standalone P2 fixture");
    append_paginated_turn(
        &codex_home,
        &project_root,
        P2,
        &fixture_uuid(0x201),
        &fixture_uuid(0x202),
        &fixture_uuid(0x203),
        "GRAPH_USER_REMOTE_UPDATE",
        "GRAPH_REMOTE_UPDATE",
    );
    let mut standalone_node = graph.nodes["p2"].clone();
    standalone_node.expected.extend(marker_turn(
        "GRAPH_USER_REMOTE_UPDATE",
        "GRAPH_REMOTE_UPDATE",
    ));
    validate_paginated_rollout(&codex_home, &standalone_node, &[P2], &project_root);
}

#[test]
#[ignore = "makes twenty-five real Codex requests; set MALLARD_RUN_LIVE_CODEX_E2E=1"]
fn live_codex_mallard_setup_push_pull_divergence() {
    if std::env::var(LIVE_TEST_GATE).ok().as_deref() != Some("1") {
        return;
    }

    // Fail before copying authentication or starting a paid request when the
    // installed CLI no longer matches the one current native rollout format
    // this clean-cutover fixture writes.
    assert_supported_codex_version();

    let source_home = std::env::var_os(SOURCE_HOME_ENV)
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
        .expect("the live test requires an authenticated source Codex home");
    let source_auth = source_home.join("auth.json");
    let source_auth_before = auth_file_state(&source_auth);
    let source_sessions_before = source_session_files(&source_home);

    let root = tempfile::tempdir().expect("create isolated live-test root");
    let process_home = root.path().join("process-home");
    let myconf1 = root.path().join("myconf1");
    let myconf2 = root.path().join("myconf2");
    let project1 = root.path().join("project1");
    let project1_copy = root.path().join("project1_copy");
    let project1_external = root.path().join("project1_external");
    let mystorage1 = root.path().join("mystorage1");
    let machine1_home = root.path().join("machine1");
    let machine2_home = root.path().join("machine2");
    for directory in [
        &process_home,
        &project1,
        &project1_copy,
        &project1_external,
        &mystorage1,
        &machine1_home,
        &machine2_home,
    ] {
        std::fs::create_dir_all(directory).expect("create isolated live-test directory");
    }
    let project1_external =
        std::fs::canonicalize(project1_external).expect("canonicalize external fixture root");
    let copied_auth1_before = copy_auth_only(&source_home, &myconf1);
    let copied_auth2_before = copy_auth_only(&source_home, &myconf2);
    let _home_guard = EnvironmentGuard::set("HOME", &process_home);
    let _userprofile_guard = EnvironmentGuard::set("USERPROFILE", &process_home);

    let repository1 = redacted(
        LocalStateRepository::from_home_dir(&machine1_home),
        "cannot create first isolated Mallard repository",
    );
    let repository2 = redacted(
        LocalStateRepository::from_home_dir(&machine2_home),
        "cannot create second isolated Mallard repository",
    );
    let service1 = ProjectSyncService::new(&repository1);
    let service2 = ProjectSyncService::new(&repository2);
    let storage = local_storage(&mystorage1);
    let storage_id = storage.id.clone();
    let mut credentials = credential_needles(&myconf1.join("auth.json"));
    credentials.extend(credential_needles(&myconf2.join("auth.json")));
    let mut path_needles = vec![
        root.path().to_string_lossy().as_bytes().to_vec(),
        source_home.to_string_lossy().as_bytes().to_vec(),
    ];
    let canonical_live_root =
        std::fs::canonicalize(root.path()).expect("canonicalize isolated live-test root");
    let canonical_live_root = canonical_live_root.to_string_lossy().as_bytes().to_vec();
    if !path_needles.contains(&canonical_live_root) {
        path_needles.push(canonical_live_root);
    }
    let mut request_count = 0;

    // Replica A creates a real thread and publishes revision 1.
    eprintln!("live Codex phase: conversation revisions 1-5");
    let first = finish_setup(
        &service1,
        &project1,
        &myconf1,
        storage.clone(),
        DraftRepositoryChoice::New,
    );
    let binding1 = first
        .binding
        .clone()
        .expect("first setup created a binding");
    let project_id1 = first.project.local_project_id.clone();
    let bundle_id = first.project.bundle_id.clone();
    prepare_fixture_parents(&project1, &IN_PROJECT_FIXTURES);
    prepare_fixture_parents(&project1_external, &OUTSIDE_PROJECT_FIXTURES);
    for path in canonical_fixture_paths(&project1, &IN_PROJECT_FIXTURES)
        .into_iter()
        .chain(canonical_fixture_paths(
            &project1_external,
            &OUTSIDE_PROJECT_FIXTURES,
        ))
    {
        assert!(!path.exists(), "revision-1 fixture already exists");
    }
    let provider_thread1 = run_codex(
        &mut request_count,
        &myconf1,
        &machine1_home,
        &project1,
        CodexRunAccess::WorkspaceWrite {
            additional_dir: &project1_external,
        },
        None,
        &revision_one_file_prompt(&project1, &project1_external),
    );
    assert_exact_fixture_tree(&project1, &IN_PROJECT_FIXTURES);
    assert_exact_fixture_tree(&project1_external, &OUTSIDE_PROJECT_FIXTURES);
    let project1_fixture_states = fixture_file_states(&project1, &IN_PROJECT_FIXTURES);
    let external_fixture_states =
        fixture_file_states(&project1_external, &OUTSIDE_PROJECT_FIXTURES);
    assert_raw_rollout_paths(
        &myconf1,
        &provider_thread1,
        &project1,
        &project1_external,
        Some(&project1_copy),
    );
    copy_in_project_fixtures(&project1, &project1_copy);
    let project1_copy_fixture_states = fixture_file_states(&project1_copy, &IN_PROJECT_FIXTURES);
    assert_eq!(
        fixture_file_states(&project1, &IN_PROJECT_FIXTURES),
        project1_fixture_states,
        "copying project fixtures changed their source files"
    );
    assert_eq!(
        fixture_file_states(&project1_external, &OUTSIDE_PROJECT_FIXTURES),
        external_fixture_states,
        "copying project fixtures changed an outside file"
    );
    let first_push = push(&service1, &project_id1, &storage_id);
    assert_eq!(first_push.revision, Some(1));
    let logical_thread = logical_thread_id(&repository1, &project_id1, &provider_thread1);
    let session_resource = mallard_domain::conversation_resource_id(&codex_key(), &logical_thread)
        .expect("derive revision-1 conversation resource ID");
    assert_portable_rollout_paths(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &logical_thread,
        &project1,
        &project1_copy,
        &project1_external,
    );
    assert_session_detail_paths(
        &service1,
        &project_id1,
        &session_resource,
        &project1,
        &myconf1,
        &provider_thread1,
        &project1_external,
    );
    assert_portable_state_is_safe(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    // Replica B links the existing repository, pulls revision 1, resumes the
    // materialized task, and publishes its branch as revision 2.
    let second = finish_setup(
        &service2,
        &project1_copy,
        &myconf2,
        storage.clone(),
        DraftRepositoryChoice::Existing {
            storage_id: storage_id.clone(),
            bundle_id: bundle_id.clone(),
            display_name: first.project.display_name.clone(),
            repository_fingerprint: first.project.repository_fingerprint.clone(),
            mismatch_acknowledged: true,
        },
    );
    let binding2 = second
        .binding
        .clone()
        .expect("second setup created a binding");
    let project_id2 = second.project.local_project_id.clone();
    let (_, initial_pull) = pull_and_apply(&service2, &storage_id, &bundle_id, &binding2);
    assert!(initial_pull.preserved_threads.is_empty());
    let provider_thread2 = provider_thread_id(&repository2, &project_id2, &logical_thread);
    assert_ne!(provider_thread2, provider_thread1);
    assert_rollout_cwd(&myconf2, &provider_thread2, &project1_copy);
    assert_raw_rollout_paths(
        &myconf2,
        &provider_thread2,
        &project1_copy,
        &project1_external,
        Some(&project1),
    );
    assert_session_detail_paths(
        &service2,
        &project_id2,
        &session_resource,
        &project1_copy,
        &myconf2,
        &provider_thread2,
        &project1_external,
    );
    assert_exact_fixture_tree(&project1_copy, &IN_PROJECT_FIXTURES);
    assert_eq!(
        fixture_file_states(&project1_external, &OUTSIDE_PROJECT_FIXTURES),
        external_fixture_states,
        "initial Push/Pull changed an outside file"
    );
    assert_fixed_point(
        &redacted(
            service1.compare_threads(&project_id1, &storage_id),
            "cannot compare replica A after initial Pull",
        ),
        1,
    );
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B after initial Pull",
        ),
        1,
    );
    let resumed2 = run_codex(
        &mut request_count,
        &myconf2,
        &machine2_home,
        &project1_copy,
        CodexRunAccess::ReadOnly,
        Some(&provider_thread2),
        "LIVE_USER_R2. Do not use tools. Reply with exactly: MALLARD_REMOTE_BRANCH",
    );
    assert_eq!(resumed2, provider_thread2);
    let second_push = push(&service2, &project_id2, &storage_id);
    assert_eq!(second_push.revision, Some(2));
    assert_portable_rollout_paths(
        &service2,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &logical_thread,
        &project1,
        &project1_copy,
        &project1_external,
    );
    assert_portable_state_is_safe(
        &service2,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    // Replica A advances from its revision-1 base. Push is gated until the
    // divergent revision-2 Pull is reviewed with Keep Both, after which the
    // retained local branch is published as revision 3.
    let resumed1 = run_codex(
        &mut request_count,
        &myconf1,
        &machine1_home,
        &project1,
        CodexRunAccess::ReadOnly,
        Some(&provider_thread1),
        "LIVE_USER_R3. Do not use tools. Reply with exactly: MALLARD_LOCAL_BRANCH",
    );
    assert_eq!(resumed1, provider_thread1);
    assert_divergent_push_gate(
        &service1,
        &project_id1,
        &storage_id,
        &logical_thread,
        "replica A",
    );
    let divergent_plan1 = redacted(
        service1.plan_bundle_restore(&storage_id, &bundle_id, &binding1),
        "cannot plan first divergent Pull",
    );
    let first_keep_both = keep_both_for(&divergent_plan1, &logical_thread);
    let divergent_apply1 = redacted(
        service1.apply_bundle_restore(
            &divergent_plan1.plan_id,
            approved_pull_actions(&divergent_plan1),
        ),
        "cannot apply first Keep Both",
    );
    assert!(divergent_apply1.success);
    assert_eq!(divergent_apply1.reviewed_base_advanced, Some(true));
    assert_keep_both_result(
        &repository1,
        &project_id1,
        &logical_thread,
        &provider_thread1,
        &first_keep_both,
        &divergent_apply1,
    );
    assert_rollout_cwd(
        &myconf1,
        &first_keep_both.remote_provider_thread_id,
        &project1,
    );
    let comparison1 = redacted(
        service1.compare_threads(&project_id1, &storage_id),
        "cannot compare after first Keep Both",
    );
    assert!(comparison1
        .entries
        .iter()
        .all(|entry| entry.state != ThreadSyncState::Diverged));
    let third_push = push(&service1, &project_id1, &storage_id);
    assert_eq!(third_push.revision, Some(3));
    assert_portable_state_is_safe(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    // Replica B pulls revision 3, retaining the stable logical primary ID
    // and materializing A's first preserved branch.
    let (_, third_pull) = pull_and_apply(&service2, &storage_id, &bundle_id, &binding2);
    assert!(third_pull.preserved_threads.is_empty());
    assert_eq!(
        provider_thread_id(&repository2, &project_id2, &logical_thread),
        provider_thread2
    );
    let first_preserved_on_b = provider_thread_id(
        &repository2,
        &project_id2,
        &first_keep_both.preserved_thread_id,
    );
    assert_rollout_cwd(&myconf2, &first_preserved_on_b, &project1_copy);

    // Starting from the shared revision-3 primary, A and B independently
    // resume their provider aliases. A publishes first as revision 4; B is
    // then correctly gated as divergent.
    let first_remote_alias = first_keep_both.remote_provider_thread_id.clone();
    let resumed4 = run_codex(
        &mut request_count,
        &myconf1,
        &machine1_home,
        &project1,
        CodexRunAccess::ReadOnly,
        Some(&first_remote_alias),
        "LIVE_USER_R4. Do not use tools. Reply with exactly: MALLARD_REVISION_FOUR",
    );
    assert_eq!(resumed4, first_remote_alias);
    let resumed5 = run_codex(
        &mut request_count,
        &myconf2,
        &machine2_home,
        &project1_copy,
        CodexRunAccess::ReadOnly,
        Some(&provider_thread2),
        "LIVE_USER_R5. Do not use tools. Reply with exactly: MALLARD_SECOND_LOCAL_BRANCH",
    );
    assert_eq!(resumed5, provider_thread2);
    let fourth_push = push(&service1, &project_id1, &storage_id);
    assert_eq!(fourth_push.revision, Some(4));
    assert_portable_state_is_safe(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    // Replica B reviews the reverse-direction divergence with Keep Both. Its
    // local branch moves to a fresh logical ID, A's revision-4 branch becomes
    // the stable primary, and both are published as revision 5 without a
    // sixth model request.
    assert_divergent_push_gate(
        &service2,
        &project_id2,
        &storage_id,
        &logical_thread,
        "replica B",
    );
    let divergent_plan2 = redacted(
        service2.plan_bundle_restore(&storage_id, &bundle_id, &binding2),
        "cannot plan second divergent Pull",
    );
    let second_keep_both = keep_both_for(&divergent_plan2, &logical_thread);
    let divergent_apply2 = redacted(
        service2.apply_bundle_restore(
            &divergent_plan2.plan_id,
            approved_pull_actions(&divergent_plan2),
        ),
        "cannot apply second Keep Both",
    );
    assert!(divergent_apply2.success);
    assert_eq!(divergent_apply2.reviewed_base_advanced, Some(true));
    assert_keep_both_result(
        &repository2,
        &project_id2,
        &logical_thread,
        &provider_thread2,
        &second_keep_both,
        &divergent_apply2,
    );
    assert_rollout_cwd(
        &myconf2,
        &second_keep_both.remote_provider_thread_id,
        &project1_copy,
    );
    assert_eq!(
        provider_thread_id(
            &repository2,
            &project_id2,
            &first_keep_both.preserved_thread_id
        ),
        first_preserved_on_b,
        "the first preserved branch changed identity during the second Keep Both"
    );
    let fifth_push = push(&service2, &project_id2, &storage_id);
    assert_eq!(fifth_push.revision, Some(5));
    assert_portable_state_is_safe(
        &service2,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    // A accepts revision 5. Both replicas are now at the same fixed point,
    // and a reviewed no-op Push from either replica must not create revision 6.
    let (_, fifth_pull) = pull_and_apply(&service1, &storage_id, &bundle_id, &binding1);
    assert!(fifth_pull.preserved_threads.is_empty());
    let second_preserved_on_a = provider_thread_id(
        &repository1,
        &project_id1,
        &second_keep_both.preserved_thread_id,
    );
    assert_rollout_cwd(&myconf1, &second_preserved_on_a, &project1);
    assert_ne!(
        first_keep_both.preserved_thread_id,
        second_keep_both.preserved_thread_id
    );
    for (repository, project_id) in [(&repository1, &project_id1), (&repository2, &project_id2)] {
        let provider_ids = [
            provider_thread_id(repository, project_id, &logical_thread),
            provider_thread_id(repository, project_id, &first_keep_both.preserved_thread_id),
            provider_thread_id(
                repository,
                project_id,
                &second_keep_both.preserved_thread_id,
            ),
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(
            provider_ids.len(),
            3,
            "fixed point collapsed distinct conversation provider identities"
        );
    }
    assert_fixed_point(
        &redacted(
            service1.compare_threads(&project_id1, &storage_id),
            "cannot compare replica A at fixed point",
        ),
        5,
    );
    assert_pull_is_no_op(&service1, &storage_id, &bundle_id, &binding1);
    assert_pull_is_no_op(&service2, &storage_id, &bundle_id, &binding2);
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B at fixed point",
        ),
        5,
    );
    for (service, project_id) in [(&service1, &project_id1), (&service2, &project_id2)] {
        let no_op = push(service, project_id, &storage_id);
        assert_eq!(no_op.revision, Some(5));
        assert_eq!(no_op.resources_changed, Some(0));
    }
    assert_eq!(
        redacted(
            service1.inspect_bundle(&storage_id, &bundle_id),
            "cannot inspect final bundle",
        )
        .head
        .revision,
        5
    );
    assert_portable_state_is_safe(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    // Revision 6: seed a five-root native 0.147 paginated DAG on replica A.
    // The exact byte/ordinal cutoffs are computed from serialized prefixes,
    // and every leaf passes the production resolver, seeder, standalone
    // builder, and projection fixpoint gate before Mallard sees it.
    eprintln!("live Codex phase: paginated graph revisions 6-10");
    let graph = write_paginated_graph(&myconf1, &project1);
    let p0_initial_bytes = read_bounded_file(
        &graph.nodes["p0"].path,
        MAX_CONVERSATION_FILE_BYTES,
        "initial P0 rollout",
    );
    let p1_initial_bytes = read_bounded_file(
        &graph.nodes["p1"].path,
        MAX_CONVERSATION_FILE_BYTES,
        "initial P1 rollout",
    );
    let sixth_push = push(&service1, &project_id1, &storage_id);
    assert_eq!(sixth_push.revision, Some(6));

    let graph_threads = [
        ("p0", logical_thread_id(&repository1, &project_id1, P0)),
        ("p1", logical_thread_id(&repository1, &project_id1, P1)),
        ("p2", logical_thread_id(&repository1, &project_id1, P2)),
        ("p3", logical_thread_id(&repository1, &project_id1, P3)),
        ("p4", logical_thread_id(&repository1, &project_id1, P4)),
    ]
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    let (_, sixth_pull) = pull_and_apply(&service2, &storage_id, &bundle_id, &binding2);
    assert!(sixth_pull.preserved_threads.is_empty());
    for name in ["p0", "p1", "p2", "p3", "p4"] {
        let logical = &graph_threads[name];
        let pulled_alias = provider_thread_id(&repository2, &project_id2, logical);
        assert_ne!(
            pulled_alias, graph.nodes[name].provider_thread_id,
            "Pull reused a source provider identity"
        );
        assert_pulled_standalone_rollout(
            &myconf2,
            &project1_copy,
            &pulled_alias,
            &graph.nodes[name].expected,
        );
    }
    assert_fixed_point(
        &redacted(
            service1.compare_threads(&project_id1, &storage_id),
            "cannot compare replica A at revision 6",
        ),
        6,
    );
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B at revision 6",
        ),
        6,
    );

    // Revision 7: A has one local-only advance and one side of a divergence;
    // B has one cloud-only advance and the other divergent side. A prepares
    // before B publishes so the reviewed head pin itself is exercised.
    append_paginated_turn(
        &myconf1,
        &project1,
        P1,
        &fixture_uuid(0x201),
        &fixture_uuid(0x202),
        &fixture_uuid(0x203),
        "GRAPH_USER_A_LOCAL_ONLY",
        "GRAPH_A_LOCAL_ONLY",
    );
    append_paginated_turn(
        &myconf1,
        &project1,
        P3,
        &fixture_uuid(0x211),
        &fixture_uuid(0x212),
        &fixture_uuid(0x213),
        "GRAPH_USER_A_DIVERGENT",
        "GRAPH_A_DIVERGENT",
    );
    let stale_a_review = redacted(
        service1.prepare_push_review(&project_id1, &storage_id),
        "cannot prepare replica A stale reviewed Push",
    );
    assert!(matches!(
        stale_a_review.destination_status,
        PushDestinationStatus::Ready { .. }
    ));

    let p2_on_b = provider_thread_id(&repository2, &project_id2, &graph_threads["p2"]);
    let p3_on_b = provider_thread_id(&repository2, &project_id2, &graph_threads["p3"]);
    append_paginated_turn(
        &myconf2,
        &project1_copy,
        &p2_on_b,
        &fixture_uuid(0x221),
        &fixture_uuid(0x222),
        &fixture_uuid(0x223),
        "GRAPH_USER_B_CLOUD_ONLY",
        "GRAPH_B_CLOUD_ONLY",
    );
    append_paginated_turn(
        &myconf2,
        &project1_copy,
        &p3_on_b,
        &fixture_uuid(0x231),
        &fixture_uuid(0x232),
        &fixture_uuid(0x233),
        "GRAPH_USER_B_DIVERGENT",
        "GRAPH_B_DIVERGENT",
    );
    let seventh_push = push(&service2, &project_id2, &storage_id);
    assert_eq!(seventh_push.revision, Some(7));
    assert_stale_push_rejected(&service1, &project_id1, &storage_id, &stale_a_review);
    let mixed_a = wait_for_thread_states(
        &service1,
        &project_id1,
        &storage_id,
        &[
            (&graph_threads["p1"], ThreadSyncState::LocalAhead),
            (&graph_threads["p2"], ThreadSyncState::StorageAhead),
            (&graph_threads["p3"], ThreadSyncState::Diverged),
        ],
        "replica A",
    );
    let p1_row = mixed_a
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&graph_threads["p1"]))
        .expect("mixed comparison lacks P1");
    assert_eq!(
        p1_row.push_decision.action,
        ConversationReviewAction::Publish
    );
    assert!(p1_row.push_decision.selectable);
    let p2_row = mixed_a
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&graph_threads["p2"]))
        .expect("mixed comparison lacks P2");
    assert_eq!(
        p2_row.pull_decision.action,
        ConversationReviewAction::MaterializeCloud
    );
    assert!(p2_row.pull_decision.selectable);
    let p3_row = mixed_a
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&graph_threads["p3"]))
        .expect("mixed comparison lacks P3");
    assert_eq!(
        p3_row.push_decision.action,
        ConversationReviewAction::RequirePull
    );
    assert!(!p3_row.push_decision.selectable);
    assert_eq!(
        p3_row.pull_decision.action,
        ConversationReviewAction::KeepBoth
    );

    let p1_on_a_before_pull = provider_thread_id(&repository1, &project_id1, &graph_threads["p1"]);
    let p3_on_a_before_pull = provider_thread_id(&repository1, &project_id1, &graph_threads["p3"]);
    let seventh_plan_a = redacted(
        service1.plan_bundle_restore(&storage_id, &bundle_id, &binding1),
        "cannot plan mixed paginated Pull on replica A",
    );
    let p3_keep_both = keep_both_for(&seventh_plan_a, &graph_threads["p3"]);
    let seventh_apply_a = redacted(
        service1.apply_bundle_restore(
            &seventh_plan_a.plan_id,
            approved_pull_actions(&seventh_plan_a),
        ),
        "cannot apply mixed paginated Pull on replica A",
    );
    assert!(seventh_apply_a.success);
    assert_eq!(seventh_apply_a.reviewed_base_advanced, Some(true));
    assert_keep_both_result(
        &repository1,
        &project_id1,
        &graph_threads["p3"],
        &p3_on_a_before_pull,
        &p3_keep_both,
        &seventh_apply_a,
    );
    assert_eq!(
        provider_thread_id(&repository1, &project_id1, &graph_threads["p1"]),
        p1_on_a_before_pull,
        "mixed Pull overwrote the unrelated LocalAhead P1"
    );

    // Revision 8 publishes A's surviving P1 advance and its preserved P3
    // branch, then B materializes both without another model request.
    let eighth_push = push(&service1, &project_id1, &storage_id);
    assert_eq!(eighth_push.revision, Some(8));
    let (_, eighth_pull) = pull_and_apply(&service2, &storage_id, &bundle_id, &binding2);
    assert!(eighth_pull.preserved_threads.is_empty());
    let p3_preserved_on_b = provider_thread_id(
        &repository2,
        &project_id2,
        &p3_keep_both.preserved_thread_id,
    );
    assert_rollout_cwd(&myconf2, &p3_preserved_on_b, &project1_copy);
    assert_fixed_point(
        &redacted(
            service1.compare_threads(&project_id1, &storage_id),
            "cannot compare replica A at revision 8",
        ),
        8,
    );
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B at revision 8",
        ),
        8,
    );

    // Revision 9 reverses the conflict direction. B also advances P0; its
    // fresh review must keep that safe local work eligible while holding P4.
    let p4_on_a = provider_thread_id(&repository1, &project_id1, &graph_threads["p4"]);
    let p4_on_b = provider_thread_id(&repository2, &project_id2, &graph_threads["p4"]);
    let p0_on_b = provider_thread_id(&repository2, &project_id2, &graph_threads["p0"]);
    append_paginated_turn(
        &myconf1,
        &project1,
        &p4_on_a,
        &fixture_uuid(0x301),
        &fixture_uuid(0x302),
        &fixture_uuid(0x303),
        "GRAPH_USER_A_REVERSE",
        "GRAPH_A_REVERSE",
    );
    append_paginated_turn(
        &myconf2,
        &project1_copy,
        &p4_on_b,
        &fixture_uuid(0x311),
        &fixture_uuid(0x312),
        &fixture_uuid(0x313),
        "GRAPH_USER_B_REVERSE",
        "GRAPH_B_REVERSE",
    );
    append_paginated_turn(
        &myconf2,
        &project1_copy,
        &p0_on_b,
        &fixture_uuid(0x321),
        &fixture_uuid(0x322),
        &fixture_uuid(0x323),
        "GRAPH_USER_B_ROOT_LOCAL",
        "GRAPH_B_ROOT_LOCAL",
    );
    let stale_b_review = redacted(
        service2.prepare_push_review(&project_id2, &storage_id),
        "cannot prepare replica B stale reviewed Push",
    );
    assert!(matches!(
        stale_b_review.destination_status,
        PushDestinationStatus::Ready { .. }
    ));
    let ninth_push = push(&service1, &project_id1, &storage_id);
    assert_eq!(ninth_push.revision, Some(9));
    assert_stale_push_rejected(&service2, &project_id2, &storage_id, &stale_b_review);
    let mixed_b = wait_for_thread_states(
        &service2,
        &project_id2,
        &storage_id,
        &[
            (&graph_threads["p0"], ThreadSyncState::LocalAhead),
            (&graph_threads["p4"], ThreadSyncState::Diverged),
        ],
        "replica B",
    );
    let p0_row = mixed_b
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&graph_threads["p0"]))
        .expect("reverse comparison lacks P0");
    assert_eq!(
        p0_row.push_decision.action,
        ConversationReviewAction::Publish
    );
    assert!(p0_row.push_decision.selectable);
    let p4_row = mixed_b
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&graph_threads["p4"]))
        .expect("reverse comparison lacks P4");
    assert_eq!(
        p4_row.push_decision.action,
        ConversationReviewAction::RequirePull
    );
    assert!(!p4_row.push_decision.selectable);
    assert_eq!(
        p4_row.pull_decision.action,
        ConversationReviewAction::KeepBoth
    );

    let ninth_plan_b = redacted(
        service2.plan_bundle_restore(&storage_id, &bundle_id, &binding2),
        "cannot plan reverse divergent paginated Pull",
    );
    let p4_keep_both = keep_both_for(&ninth_plan_b, &graph_threads["p4"]);
    let ninth_apply_b = redacted(
        service2.apply_bundle_restore(&ninth_plan_b.plan_id, approved_pull_actions(&ninth_plan_b)),
        "cannot apply reverse Keep Both",
    );
    assert!(ninth_apply_b.success);
    assert_eq!(ninth_apply_b.reviewed_base_advanced, Some(true));
    assert_keep_both_result(
        &repository2,
        &project_id2,
        &graph_threads["p4"],
        &p4_on_b,
        &p4_keep_both,
        &ninth_apply_b,
    );
    assert_eq!(
        provider_thread_id(&repository2, &project_id2, &graph_threads["p0"]),
        p0_on_b,
        "reverse divergent Pull overwrote the unrelated LocalAhead P0"
    );

    // Revision 10 publishes B's P0 and preserved P4 work; A pulls it and the
    // complete project conversation set reaches a stable no-op fixed point.
    let tenth_push = push(&service2, &project_id2, &storage_id);
    assert_eq!(tenth_push.revision, Some(10));
    let (_, tenth_pull) = pull_and_apply(&service1, &storage_id, &bundle_id, &binding1);
    assert!(tenth_pull.preserved_threads.is_empty());
    assert_fixed_point(
        &redacted(
            service1.compare_threads(&project_id1, &storage_id),
            "cannot compare replica A at revision 10",
        ),
        10,
    );
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B at revision 10",
        ),
        10,
    );
    let storage_before_fixed_point_noops = storage_digest_inventory(&mystorage1);
    assert_pull_is_no_op(&service1, &storage_id, &bundle_id, &binding1);
    assert_pull_is_no_op(&service2, &storage_id, &bundle_id, &binding2);
    for (service, project_id) in [(&service1, &project_id1), (&service2, &project_id2)] {
        let no_op = push(service, project_id, &storage_id);
        assert_eq!(no_op.revision, Some(10));
        assert_eq!(no_op.resources_changed, Some(0));
    }
    assert_eq!(
        storage_digest_inventory(&mystorage1),
        storage_before_fixed_point_noops,
        "fixed-point project Push/Pull created or replaced storage objects"
    );
    let project_before_library = redacted(
        service1.inspect_bundle(&storage_id, &bundle_id),
        "cannot inspect revision-10 bundle",
    );
    assert_eq!(project_before_library.head.revision, 10);
    let project_repository_root = mystorage1.join(".mallard/v1/repositories");
    let project_objects_before_library = storage_snapshot(&project_repository_root);

    // Exercise skills and both Codex pet formats through the production
    // storage-Library service in the same two machine/provider homes and the
    // same local-folder store. Library repositories are independent of the
    // project bundle and must not advance its revision-10 head.
    eprintln!("live Codex phase: storage Library skills and pets");
    let catalog_user1 = root.path().join("catalog-user1");
    let catalog_user2 = root.path().join("catalog-user2");
    let mut catalog_privacy_needles = credentials.clone();
    catalog_privacy_needles.extend(path_needles.clone());
    catalog_privacy_needles.push(catalog_user1.to_string_lossy().as_bytes().to_vec());
    catalog_privacy_needles.push(catalog_user2.to_string_lossy().as_bytes().to_vec());
    let catalog_report = run_catalog_flow(CatalogFlowInputs {
        machine_home_a: &machine1_home,
        machine_home_b: &machine2_home,
        user_home_a: &catalog_user1,
        user_home_b: &catalog_user2,
        codex_home_a: &myconf1,
        codex_home_b: &myconf2,
        storage: &storage,
        privacy_needles: &catalog_privacy_needles,
    });
    assert_eq!(catalog_report.repository_count, 8);
    assert_eq!(catalog_report.primary_revisions, vec![4, 4, 4, 4]);
    assert_eq!(catalog_report.preserved_copy_revisions, vec![1, 1, 1, 1]);
    assert_eq!(catalog_report.native_provider_process_calls, 0);
    assert!(catalog_report.injected_inventory_calls > 0);
    let project_after_library = redacted(
        service1.inspect_bundle(&storage_id, &bundle_id),
        "cannot inspect project bundle after Library flow",
    );
    assert_eq!(
        project_after_library.head, project_before_library.head,
        "storage-Library activity changed the project bundle head coordinate"
    );
    assert_eq!(
        project_after_library.manifest, project_before_library.manifest,
        "storage-Library activity changed the revision-10 project manifest"
    );
    assert_eq!(
        storage_snapshot(&project_repository_root),
        project_objects_before_library,
        "storage-Library activity changed a project repository object"
    );

    // Native descendants remain pinned to their exact serialized prefixes:
    // later P1 and P0 appends must not leak into P3/P4 or invalidate a base.
    let p0_after = read_bounded_file(
        &graph.nodes["p0"].path,
        MAX_CONVERSATION_FILE_BYTES,
        "final native P0 rollout",
    );
    let p1_after = read_bounded_file(
        &graph.nodes["p1"].path,
        MAX_CONVERSATION_FILE_BYTES,
        "final native P1 rollout",
    );
    assert!(p0_after.starts_with(&p0_initial_bytes));
    assert!(p1_after.starts_with(&p1_initial_bytes));
    for (name, segments) in [
        ("p1", vec![P0, P1]),
        ("p3", vec![P0, P1, P3]),
        ("p4", vec![P0, P1, P4]),
    ] {
        let mut node = graph.nodes[name].clone();
        node.expected = match name {
            "p1" => graph.nodes[name]
                .expected
                .iter()
                .cloned()
                .chain(marker_turn("GRAPH_USER_A_LOCAL_ONLY", "GRAPH_A_LOCAL_ONLY"))
                .collect(),
            "p3" => graph.nodes[name]
                .expected
                .iter()
                .cloned()
                .chain(marker_turn("GRAPH_USER_A_DIVERGENT", "GRAPH_A_DIVERGENT"))
                .collect(),
            "p4" => graph.nodes[name]
                .expected
                .iter()
                .cloned()
                .chain(marker_turn("GRAPH_USER_A_REVERSE", "GRAPH_A_REVERSE"))
                .collect(),
            _ => graph.nodes[name].expected.clone(),
        };
        validate_paginated_rollout(&myconf1, &node, &segments, &project1);
    }
    let mut p2_final_expected = graph.nodes["p2"].expected.clone();
    p2_final_expected.extend(marker_turn("GRAPH_USER_B_CLOUD_ONLY", "GRAPH_B_CLOUD_ONLY"));
    assert_pulled_standalone_rollout(&myconf1, &project1, P2, &p2_final_expected);

    let mut existing_primary = marker_turn("LIVE_USER_R1", "MALLARD_BASE");
    existing_primary.extend(marker_turn("LIVE_USER_R2", "MALLARD_REMOTE_BRANCH"));
    existing_primary.extend(marker_turn("LIVE_USER_R4", "MALLARD_REVISION_FOUR"));
    let mut first_preserved_history = marker_turn("LIVE_USER_R1", "MALLARD_BASE");
    first_preserved_history.extend(marker_turn("LIVE_USER_R3", "MALLARD_LOCAL_BRANCH"));
    let mut second_preserved_history = marker_turn("LIVE_USER_R1", "MALLARD_BASE");
    second_preserved_history.extend(marker_turn("LIVE_USER_R2", "MALLARD_REMOTE_BRANCH"));
    second_preserved_history.extend(marker_turn("LIVE_USER_R5", "MALLARD_SECOND_LOCAL_BRANCH"));
    let mut p0_history = graph.nodes["p0"].expected.clone();
    p0_history.extend(marker_turn("GRAPH_USER_B_ROOT_LOCAL", "GRAPH_B_ROOT_LOCAL"));
    let mut p1_history = graph.nodes["p1"].expected.clone();
    p1_history.extend(marker_turn("GRAPH_USER_A_LOCAL_ONLY", "GRAPH_A_LOCAL_ONLY"));
    let p2_history = p2_final_expected;
    let mut p3_primary_history = graph.nodes["p3"].expected.clone();
    p3_primary_history.extend(marker_turn("GRAPH_USER_B_DIVERGENT", "GRAPH_B_DIVERGENT"));
    let mut p3_preserved_history = graph.nodes["p3"].expected.clone();
    p3_preserved_history.extend(marker_turn("GRAPH_USER_A_DIVERGENT", "GRAPH_A_DIVERGENT"));
    let mut p4_primary_history = graph.nodes["p4"].expected.clone();
    p4_primary_history.extend(marker_turn("GRAPH_USER_A_REVERSE", "GRAPH_A_REVERSE"));
    let mut p4_preserved_history = graph.nodes["p4"].expected.clone();
    p4_preserved_history.extend(marker_turn("GRAPH_USER_B_REVERSE", "GRAPH_B_REVERSE"));
    let histories = vec![
        (logical_thread.clone(), existing_primary),
        (
            first_keep_both.preserved_thread_id.clone(),
            first_preserved_history,
        ),
        (
            second_keep_both.preserved_thread_id.clone(),
            second_preserved_history,
        ),
        (graph_threads["p0"].clone(), p0_history),
        (graph_threads["p1"].clone(), p1_history),
        (graph_threads["p2"].clone(), p2_history),
        (graph_threads["p3"].clone(), p3_primary_history),
        (
            p3_keep_both.preserved_thread_id.clone(),
            p3_preserved_history,
        ),
        (graph_threads["p4"].clone(), p4_primary_history),
        (
            p4_keep_both.preserved_thread_id.clone(),
            p4_preserved_history,
        ),
    ];
    assert_eq!(histories.len(), 10);
    let r1_derived_threads = [
        logical_thread.clone(),
        first_keep_both.preserved_thread_id.clone(),
        second_keep_both.preserved_thread_id.clone(),
    ];
    for (service, repository, project_id, codex_home, project_root, forbidden_root) in [
        (
            &service1,
            &repository1,
            &project_id1,
            myconf1.as_path(),
            project1.as_path(),
            project1_copy.as_path(),
        ),
        (
            &service2,
            &repository2,
            &project_id2,
            myconf2.as_path(),
            project1_copy.as_path(),
            project1.as_path(),
        ),
    ] {
        for logical in &r1_derived_threads {
            let provider_id = provider_thread_id(repository, project_id, logical);
            let resource_id = mallard_domain::conversation_resource_id(&codex_key(), logical)
                .expect("derive final R1-derived resource ID");
            assert_raw_rollout_paths(
                codex_home,
                &provider_id,
                project_root,
                &project1_external,
                Some(forbidden_root),
            );
            assert_session_detail_paths(
                service,
                project_id,
                &resource_id,
                project_root,
                codex_home,
                &provider_id,
                &project1_external,
            );
        }
    }
    eprintln!("live Codex phase: twenty disposable headless history resumes");
    verify_all_histories(
        &mut request_count,
        [
            (&repository1, &project_id1, &myconf1, &project1),
            (&repository2, &project_id2, &myconf2, &project1_copy),
        ],
        &histories,
    );
    assert_eq!(
        request_count, EXPECTED_MODEL_REQUESTS,
        "live scenario changed its Luna-low model-request budget"
    );
    assert_exact_fixture_tree(&project1, &IN_PROJECT_FIXTURES);
    assert_exact_fixture_tree(&project1_copy, &IN_PROJECT_FIXTURES);
    assert_exact_fixture_tree(&project1_external, &OUTSIDE_PROJECT_FIXTURES);
    assert_eq!(
        fixture_file_states(&project1, &IN_PROJECT_FIXTURES),
        project1_fixture_states,
        "the completed live flow changed source project fixtures"
    );
    assert_eq!(
        fixture_file_states(&project1_copy, &IN_PROJECT_FIXTURES),
        project1_copy_fixture_states,
        "the completed live flow changed copied project fixtures"
    );
    assert_eq!(
        fixture_file_states(&project1_external, &OUTSIDE_PROJECT_FIXTURES),
        external_fixture_states,
        "the completed live flow changed outside project fixtures"
    );
    eprintln!("live Codex phase: complete ({request_count} Luna-low requests)");
    assert_portable_state_is_safe(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    let copied_auth1_after = auth_file_state(&myconf1.join("auth.json"));
    let copied_auth2_after = auth_file_state(&myconf2.join("auth.json"));
    let source_auth_after = auth_file_state(&source_auth);
    let source_sessions_after = source_session_files(&source_home);
    assert!(
        copied_auth1_before == copied_auth1_after,
        "first copied authentication file changed during the live test"
    );
    assert!(
        copied_auth2_before == copied_auth2_after,
        "second copied authentication file changed during the live test"
    );
    assert!(
        source_auth_before == source_auth_after,
        "source authentication metadata or content changed during the live test"
    );
    assert!(
        source_sessions_before == source_sessions_after,
        "the live test created a session in the source Codex home"
    );
}
