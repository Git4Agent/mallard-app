//! Opt-in end-to-end coverage for two isolated Claude Code homes
//! synchronizing through one real Mallard local-folder repository.
//!
//! The credentialless target owns one strict loopback aimock process, one
//! bounded Claude version probe, and twelve public Claude model-turn CLI
//! processes. It creates five source revisions, resolves Keep Both in both
//! directions, reopens all three final histories from both replicas, resets
//! and republishes storage, and proves one incremental cursor append.
//! A separately gated paid Tuzi target reuses the production Mallard oracles
//! for a smaller three-turn create, relocated-resume, divergent-Pull, and
//! storage-reset flow.
//! Every filesystem root, provider home, and credential-shaped value used by
//! either fixture is disposable. Both tests are ignored and independently
//! gated so neither can contact an account or inspect a real Claude profile
//! by default.

use mallard_domain::{
    ActionId, BundleId, CloudThreadId, ConversationMaterializationState, ConversationReviewAction,
    DraftProfileSelection, DraftRepositoryChoice, DraftStorageSelection, LocalConversationBinding,
    LocalProjectId, ProjectBinding, ProjectSetupDraft, ProviderKey, ResourceId, RestoreActionKind,
    RestorePlan, StorageConfig, StorageId, ThreadId, MAX_CONVERSATION_FILE_BYTES,
};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::project_activity::{ProjectSessionSummary, SessionSourceState};
use mallard_infrastructure::providers::claude::{encode_claude_project_path, ClaudeAdapter};
use mallard_infrastructure::providers::{
    SessionAdapter, SessionCountQuality, SessionTimelineContext, SessionTimelineEventKind,
    SessionTimelineQuery, SessionToolCallStatus, SessionToolCategory,
};
use mallard_infrastructure::session_path_map::SessionPathMapper;
use mallard_infrastructure::storage_providers::local::local_storage_config;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri_app_lib::{
    ProjectActivity, ProjectDetail, ProjectOperationResult, ProjectSessionTimelineRequest,
    ProjectSyncService, PushApproval, PushDestinationPullReason, PushDestinationStatus,
    RestoreResult, SessionDetailErrorCode, StorageDataClearResult, ThreadSyncComparison,
    ThreadSyncState,
};
use walkdir::WalkDir;

const LIVE_TEST_GATE: &str = "MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E";
const MODEL: &str = "claude-sonnet-4-6";
const LIVE_TUZI_TEST_GATE: &str = "MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E";
const LIVE_TUZI_MODEL: &str = "claude-sonnet-5";
const LIVE_TUZI_SOURCE_SESSION_ID: &str = "018f3f2e-7b91-7ad0-8000-000000000301";
const EXPECTED_AIMOCK_CLAUDE_VERSION: &str = "2.1.266 (Claude Code)";
const EXPECTED_TUZI_CLAUDE_VERSION: &str = "2.1.266 (Claude Code)";
const PLACEHOLDER_API_KEY: &str = "mallard-local-aimock-placeholder";
const INHERITED_SECRET_MARKER: &str = "mallard-inherited-secret-sentinel";
const READ_TOOL_CALL_ID: &str = "mallard_claude_read_r1";
const READ_FILE_MARKER: &str = "MALLARD_CLAUDE_READ_FILE";
const FOREIGN_DECOY_SESSION_ID: &str = "018f3f2e-7b91-7ad0-8000-000000000299";
/// Fake credentials planted into the Read-tool file: shaped so the
/// mallard.redact.v1 layers detect them (provider prefix; credential
/// key/value), proving live cloud-object cleanliness end to end.
const PLANTED_LIVE_ANTHROPIC_KEY: &str = "sk-ant-api03-fakelivecorpusaaaabbbb";
const PLANTED_LIVE_PASSWORD: &str = "hunter2fakelive";
const SOURCE_SESSION_ID: &str = "018f3f2e-7b91-7ad0-8000-000000000201";
const FINAL_REOPEN_ACK: &str = "MALLARD_CLAUDE_REOPEN_CAPTURED";

const EXPECTED_SOURCE_PROCESSES: usize = 5;
const EXPECTED_REOPEN_PROCESSES: usize = 6;
const EXPECTED_MODEL_TURN_PROCESSES: usize = EXPECTED_SOURCE_PROCESSES + EXPECTED_REOPEN_PROCESSES;
/// One more real resumed turn proves the checkpoint cursor extends over a
/// live CLI append.
const EXPECTED_CURSOR_APPEND_PROCESSES: usize = EXPECTED_MODEL_TURN_PROCESSES + 1;
const EXPECTED_AIMOCK_REQUESTS: usize = 13;
const EXPECTED_AIMOCK_RAW_REQUESTS: usize =
    EXPECTED_AIMOCK_REQUESTS + EXPECTED_CURSOR_APPEND_PROCESSES;

const MAX_COMMAND_ARGUMENTS: usize = 32;
const MAX_COMMAND_ARGUMENT_BYTES: usize = 4096;
const MAX_PROMPT_BYTES: usize = 512;
const MAX_PROCESS_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_CONTROL_BYTES: u64 = 512 * 1024;
const MAX_PROVIDER_DOCUMENT_BYTES: u64 = 4 * 1024;
const MAX_PROVIDER_KEY_BYTES: usize = 1_024;
const MAX_STORAGE_OBJECT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TREE_ENTRIES: usize = 20_000;
const MAX_TIMELINE_EVENTS: usize = 1024;
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(60);
const LIVE_TUZI_CLAUDE_TIMEOUT: Duration = Duration::from_secs(210);
const MAX_LIVE_TUZI_COST_PER_TURN_USD: f64 = 0.05;
const EXPECTED_LIVE_TUZI_PROCESSES: usize = 3;
const AIMOCK_START_TIMEOUT: Duration = Duration::from_secs(30);
const AIMOCK_STOP_TIMEOUT: Duration = Duration::from_secs(30);
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const TRANSCRIPT_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

const CREDENTIAL_ENV_KEYS: &[&str] = &[
    "AIMOCK_DUMMY_KEY_MARKER",
    "AIMOCK_PROVIDER_ANTHROPIC_KEY",
    "AIMOCK_PROVIDER_AZURE_KEY",
    "AIMOCK_PROVIDER_COHERE_KEY",
    "AIMOCK_PROVIDER_ELEVENLABS_KEY",
    "AIMOCK_PROVIDER_FAL_KEY",
    "AIMOCK_PROVIDER_GEMINI_KEY",
    "AIMOCK_PROVIDER_GROK_KEY",
    "AIMOCK_PROVIDER_OLLAMA_KEY",
    "AIMOCK_PROVIDER_OPENAI_KEY",
    "AIMOCK_PROVIDER_OPENROUTER_KEY",
    "AIMOCK_PROVIDER_VEO_KEY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_BEDROCK_BASE_URL",
    "ANTHROPIC_BETAS",
    "ANTHROPIC_CUSTOM_HEADERS",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_FOUNDRY_BASE_URL",
    "ANTHROPIC_FOUNDRY_RESOURCE",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_VERTEX_BASE_URL",
    "ANTHROPIC_VERTEX_PROJECT_ID",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AZURE_OPENAI_API_KEY",
    "AZURE_OPENAI_ENDPOINT",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CONFIG_DIR",
    "CODEX_API_KEY",
    "CODEX_HOME",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORGANIZATION",
    "OPENAI_ORG_ID",
    "OPENAI_PROJECT_ID",
];

const PROXY_ENV_KEYS: &[&str] = &[
    "ALL_PROXY",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "all_proxy",
    "https_proxy",
    "http_proxy",
];

fn redacted<T, E>(result: Result<T, E>, message: &'static str) -> T {
    match result {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

fn claude_key() -> ProviderKey {
    ProviderKey::parse("claude").expect("static Claude provider key")
}

fn secure_directory(path: &Path) {
    std::fs::create_dir_all(path).expect("create isolated test directory");
    let metadata = std::fs::symlink_metadata(path).expect("inspect isolated test directory");
    assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .expect("secure isolated test directory");
    }
}

fn write_private_new(path: &Path, bytes: &[u8]) {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).expect("create private fixture file");
    file.write_all(bytes).expect("write private fixture file");
    file.flush().expect("flush private fixture file");
}

fn read_bounded(path: &Path, limit: u64, label: &str) -> Vec<u8> {
    let metadata =
        std::fs::symlink_metadata(path).unwrap_or_else(|_| panic!("cannot inspect {label}"));
    assert!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= limit,
        "{label} is not a bounded no-follow regular file"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    std::fs::File::open(path)
        .unwrap_or_else(|_| panic!("cannot open {label}"))
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .unwrap_or_else(|_| panic!("cannot read {label}"));
    assert!(
        bytes.len() as u64 <= limit,
        "{label} exceeds its read bound"
    );
    bytes
}

fn byte_slice_contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Positive control for exclusion scans: the needle must appear in at least
/// one storage object body, proving the scan reads real plaintext objects.
fn assert_storage_contains(storage_root: &Path, needle: &[u8]) {
    let mut found = false;
    for entry in WalkDir::new(storage_root).follow_links(false) {
        let entry = redacted(entry, "cannot inspect Claude local-folder storage");
        if !entry.file_type().is_file() {
            continue;
        }
        let bytes = read_bounded(
            entry.path(),
            MAX_STORAGE_OBJECT_BYTES,
            "Claude storage object",
        );
        if bytes
            .windows(needle.len().max(1))
            .any(|window| window == needle)
        {
            found = true;
            break;
        }
    }
    assert!(
        found,
        "no storage object contains the expected needle of length {}",
        needle.len()
    );
}

fn assert_excludes(haystack: &[u8], needles: &[Vec<u8>], label: &str) {
    assert!(
        needles
            .iter()
            .all(|needle| !byte_slice_contains(haystack, needle)),
        "{label} contains protected test material"
    );
}

fn identity_needles<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<Vec<u8>> {
    values
        .into_iter()
        .filter(|value| !value.is_empty())
        .map(|value| value.as_bytes().to_vec())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn closed_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let path = std::env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
    let mut command = Command::new(program);
    command.env_clear().env("PATH", path);
    command
}

fn remove_sensitive_environment(command: &mut Command) {
    for key in CREDENTIAL_ENV_KEYS
        .iter()
        .chain(PROXY_ENV_KEYS.iter())
        .copied()
    {
        command.env_remove(key);
    }
    command
        .env("NO_PROXY", "127.0.0.1")
        .env("no_proxy", "127.0.0.1");
}

fn assert_supported_claude_version(process_home: &Path, expected_version: &str) {
    let mut command = closed_command("claude");
    remove_sensitive_environment(&mut command);
    let mut child = command
        .arg("--version")
        .current_dir(process_home)
        .env("HOME", process_home)
        .env("USERPROFILE", process_home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the Claude Code version probe");
    let stdout = child.stdout.take().expect("capture Claude version stdout");
    let stderr = child.stderr.take().expect("capture Claude version stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(257).read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.take(1025).read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + VERSION_TIMEOUT;
    loop {
        if child
            .try_wait()
            .expect("wait for Claude Code version probe")
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            panic!("Claude Code version probe exceeded its timeout");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let status = child.wait().expect("finish Claude Code version probe");
    let stdout = stdout_reader
        .join()
        .expect("Claude version stdout reader panicked")
        .expect("collect Claude version stdout");
    let stderr = stderr_reader
        .join()
        .expect("Claude version stderr reader panicked")
        .expect("collect Claude version stderr");
    assert!(status.success(), "Claude Code version probe failed");
    assert!(
        stdout.len() <= 256 && stderr.len() <= 1024,
        "Claude Code version output exceeds its bound"
    );
    assert_eq!(
        String::from_utf8(stdout)
            .expect("Claude Code version output must be UTF-8")
            .trim(),
        expected_version,
        "Claude Code changed; re-prove the native transcript contract before updating this gate"
    );
    assert!(stderr.is_empty(), "Claude Code version probe wrote stderr");
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

#[derive(Clone)]
struct LocalAimockProvider {
    base_url: String,
}

impl LocalAimockProvider {
    fn new(port: u16) -> Self {
        assert_ne!(port, 0);
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveTuziProviderDocument {
    #[serde(rename = "_type")]
    connection_type: String,
    key: String,
    url: String,
}

struct LiveTuziConnection {
    api_key: String,
    base_url: String,
}

impl LiveTuziConnection {
    fn protected_needles(&self) -> Vec<Vec<u8>> {
        vec![
            self.api_key.as_bytes().to_vec(),
            INHERITED_SECRET_MARKER.as_bytes().to_vec(),
        ]
    }
}

fn normalize_tuzi_url(value: &str) -> Result<String, &'static str> {
    if value.is_empty()
        || value.len() > 2_048
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err("invalid_provider_url");
    }
    let url = tauri::Url::parse(value).map_err(|_| "invalid_provider_url")?;
    if url.scheme() != "https"
        || url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str() != Some("api.tu-zi.com")
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "/" | "/v1")
    {
        return Err("invalid_provider_url");
    }
    Ok(format!("{}://api.tu-zi.com", url.scheme()))
}

fn parse_live_tuzi_connection(bytes: &[u8]) -> Result<LiveTuziConnection, &'static str> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_PROVIDER_DOCUMENT_BYTES {
        return Err("invalid_provider_document");
    }
    let document: LiveTuziProviderDocument =
        serde_json::from_slice(bytes).map_err(|_| "invalid_provider_document")?;
    if document.connection_type != "newapi_channel_conn"
        || document.key.is_empty()
        || document.key.len() > MAX_PROVIDER_KEY_BYTES
        || document.key.trim() != document.key
        || document.key.chars().any(char::is_control)
    {
        return Err("invalid_provider_document");
    }
    Ok(LiveTuziConnection {
        api_key: document.key,
        base_url: normalize_tuzi_url(&document.url)?,
    })
}

fn read_live_tuzi_provider_document(path: &Path) -> Vec<u8> {
    let before =
        std::fs::symlink_metadata(path).expect("live Claude provider configuration is unavailable");
    assert!(
        before.is_file() && !before.file_type().is_symlink(),
        "live Claude provider configuration must be a no-follow regular file"
    );
    assert!(
        before.len() > 0 && before.len() <= MAX_PROVIDER_DOCUMENT_BYTES,
        "live Claude provider configuration exceeds its safety bound"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
        assert_eq!(
            before.mode() & 0o7777,
            0o600,
            "live Claude provider configuration must have Unix mode 0600"
        );
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .expect("live Claude provider configuration is unreadable");
        let opened = file
            .metadata()
            .expect("inspect opened live Claude provider configuration");
        assert!(
            opened.is_file()
                && opened.dev() == before.dev()
                && opened.ino() == before.ino()
                && opened.mode() == before.mode()
                && opened.len() == before.len()
                && opened.mtime() == before.mtime()
                && opened.mtime_nsec() == before.mtime_nsec()
                && opened.ctime() == before.ctime()
                && opened.ctime_nsec() == before.ctime_nsec(),
            "live Claude provider configuration changed while opening"
        );
        let mut bytes = Vec::with_capacity(opened.len() as usize);
        (&mut file)
            .take(MAX_PROVIDER_DOCUMENT_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .expect("live Claude provider configuration is unreadable");
        let after_open = file
            .metadata()
            .expect("reinspect opened live Claude provider configuration");
        let after_path = std::fs::symlink_metadata(path)
            .expect("reinspect live Claude provider configuration path");
        assert!(
            bytes.len() as u64 == opened.len()
                && bytes.len() as u64 <= MAX_PROVIDER_DOCUMENT_BYTES
                && after_open.dev() == opened.dev()
                && after_open.ino() == opened.ino()
                && after_open.mode() == opened.mode()
                && after_open.len() == opened.len()
                && after_open.mtime() == opened.mtime()
                && after_open.mtime_nsec() == opened.mtime_nsec()
                && after_open.ctime() == opened.ctime()
                && after_open.ctime_nsec() == opened.ctime_nsec()
                && after_path.dev() == after_open.dev()
                && after_path.ino() == after_open.ino()
                && after_path.mode() == after_open.mode()
                && after_path.len() == after_open.len()
                && after_path.mtime() == after_open.mtime()
                && after_path.mtime_nsec() == after_open.mtime_nsec()
                && after_path.ctime() == after_open.ctime()
                && after_path.ctime_nsec() == after_open.ctime_nsec(),
            "live Claude provider configuration changed while reading"
        );
        bytes
    }
    #[cfg(not(unix))]
    read_bounded(
        path,
        MAX_PROVIDER_DOCUMENT_BYTES,
        "live Claude provider configuration",
    )
}

fn load_live_tuzi_connection() -> LiveTuziConnection {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository_root = manifest_dir
        .parent()
        .expect("Tauri manifest directory must have a repository parent");
    parse_live_tuzi_connection(&read_live_tuzi_provider_document(
        &repository_root.join(".env.local"),
    ))
    .expect("live Claude provider configuration is missing or invalid")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum HistoryRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct HistoryMarker {
    role: HistoryRole,
    marker: String,
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

fn final_history_oracles() -> Vec<Vec<HistoryMarker>> {
    let mut primary = marker_turn("MALLARD_CLAUDE_USER_R1", "MALLARD_CLAUDE_BASE");
    primary.extend(marker_turn(
        "MALLARD_CLAUDE_USER_R2",
        "MALLARD_CLAUDE_REMOTE_BRANCH",
    ));
    primary.extend(marker_turn(
        "MALLARD_CLAUDE_USER_R4",
        "MALLARD_CLAUDE_REVISION_FOUR",
    ));

    let mut preserved_a = marker_turn("MALLARD_CLAUDE_USER_R1", "MALLARD_CLAUDE_BASE");
    preserved_a.extend(marker_turn(
        "MALLARD_CLAUDE_USER_R3",
        "MALLARD_CLAUDE_LOCAL_BRANCH",
    ));

    let mut preserved_b = marker_turn("MALLARD_CLAUDE_USER_R1", "MALLARD_CLAUDE_BASE");
    preserved_b.extend(marker_turn(
        "MALLARD_CLAUDE_USER_R2",
        "MALLARD_CLAUDE_REMOTE_BRANCH",
    ));
    preserved_b.extend(marker_turn(
        "MALLARD_CLAUDE_USER_R5",
        "MALLARD_CLAUDE_SECOND_LOCAL_BRANCH",
    ));
    vec![primary, preserved_a, preserved_b]
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct AimockScenarioDocument {
    version: u64,
    model: String,
    expected_request_count: usize,
    expected_probe_request_count: usize,
    scenarios: Vec<AimockScenario>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct AimockScenario {
    id: String,
    prompt_marker: String,
    expected_history: Vec<HistoryMarker>,
    require_tool_result: bool,
    expected_tool_call_id: Option<String>,
    expected_tool_result_marker: Option<String>,
    response: AimockResponse,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AimockResponse {
    Text { content: String },
    ReadTool { call_id: String, file_path: String },
}

fn text_scenario(
    id: impl Into<String>,
    prompt: impl Into<String>,
    history: Vec<HistoryMarker>,
    response: impl Into<String>,
) -> AimockScenario {
    AimockScenario {
        id: id.into(),
        prompt_marker: prompt.into(),
        expected_history: history,
        require_tool_result: false,
        expected_tool_call_id: None,
        expected_tool_result_marker: None,
        response: AimockResponse::Text {
            content: response.into(),
        },
    }
}

fn aimock_scenario_document(read_marker_path: &Path) -> AimockScenarioDocument {
    let mut scenarios = vec![
        AimockScenario {
            id: "source_r1_read".to_string(),
            prompt_marker: "MALLARD_CLAUDE_USER_R1".to_string(),
            expected_history: Vec::new(),
            require_tool_result: false,
            expected_tool_call_id: None,
            expected_tool_result_marker: None,
            response: AimockResponse::ReadTool {
                call_id: READ_TOOL_CALL_ID.to_string(),
                file_path: read_marker_path.to_string_lossy().into_owned(),
            },
        },
        AimockScenario {
            id: "source_r1_final".to_string(),
            prompt_marker: "MALLARD_CLAUDE_USER_R1".to_string(),
            expected_history: Vec::new(),
            require_tool_result: true,
            expected_tool_call_id: Some(READ_TOOL_CALL_ID.to_string()),
            expected_tool_result_marker: Some(READ_FILE_MARKER.to_string()),
            response: AimockResponse::Text {
                content: "MALLARD_CLAUDE_BASE".to_string(),
            },
        },
        text_scenario(
            "source_r2",
            "MALLARD_CLAUDE_USER_R2",
            marker_turn("MALLARD_CLAUDE_USER_R1", "MALLARD_CLAUDE_BASE"),
            "MALLARD_CLAUDE_REMOTE_BRANCH",
        ),
        text_scenario(
            "source_r3",
            "MALLARD_CLAUDE_USER_R3",
            marker_turn("MALLARD_CLAUDE_USER_R1", "MALLARD_CLAUDE_BASE"),
            "MALLARD_CLAUDE_LOCAL_BRANCH",
        ),
        text_scenario(
            "source_r4",
            "MALLARD_CLAUDE_USER_R4",
            marker_turn("MALLARD_CLAUDE_USER_R1", "MALLARD_CLAUDE_BASE")
                .into_iter()
                .chain(marker_turn(
                    "MALLARD_CLAUDE_USER_R2",
                    "MALLARD_CLAUDE_REMOTE_BRANCH",
                ))
                .collect(),
            "MALLARD_CLAUDE_REVISION_FOUR",
        ),
        text_scenario(
            "source_r5",
            "MALLARD_CLAUDE_USER_R5",
            marker_turn("MALLARD_CLAUDE_USER_R1", "MALLARD_CLAUDE_BASE")
                .into_iter()
                .chain(marker_turn(
                    "MALLARD_CLAUDE_USER_R2",
                    "MALLARD_CLAUDE_REMOTE_BRANCH",
                ))
                .collect(),
            "MALLARD_CLAUDE_SECOND_LOCAL_BRANCH",
        ),
    ];
    for replica in 0..2 {
        for (history_index, history) in final_history_oracles().into_iter().enumerate() {
            scenarios.push(text_scenario(
                format!("verify_{replica}_{history_index}"),
                format!("MALLARD_CLAUDE_USER_VERIFY_{replica}_{history_index}"),
                history,
                FINAL_REOPEN_ACK,
            ));
        }
    }
    scenarios.push(text_scenario(
        "cursor_append",
        "MALLARD_CLAUDE_USER_CURSOR_APPEND",
        final_history_oracles()
            .into_iter()
            .next()
            .expect("primary final history oracle"),
        "MALLARD_CLAUDE_CURSOR_EXTENDED",
    ));
    assert_eq!(scenarios.len(), EXPECTED_AIMOCK_REQUESTS);
    assert_eq!(
        scenarios
            .iter()
            .map(|scenario| scenario.id.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        scenarios.len(),
        "aimock scenario IDs must be unique"
    );
    AimockScenarioDocument {
        version: 1,
        model: MODEL.to_string(),
        expected_request_count: EXPECTED_AIMOCK_REQUESTS,
        expected_probe_request_count: EXPECTED_CURSOR_APPEND_PROCESSES,
        scenarios,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AimockReadiness {
    #[serde(rename = "type")]
    kind: String,
    version: u64,
    port: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AimockAudit {
    version: u64,
    expected_request_count: usize,
    expected_probe_request_count: usize,
    request_count: usize,
    strict_mode: bool,
    loopback_only: bool,
    recording_absent: bool,
    upstream_absent: bool,
    inbound_auth_required: bool,
    fixed_placeholder_enforced: bool,
    raw_request_count: usize,
    raw_expected_request_count: usize,
    raw_model_request_count: usize,
    raw_probe_request_count: usize,
    raw_unexpected_request_count: usize,
    raw_wrong_method_count: usize,
    raw_wrong_path_count: usize,
    raw_auth_rejection_count: usize,
    raw_authorization_header_count: usize,
    raw_alternate_credential_header_count: usize,
    raw_upgrade_request_count: usize,
    raw_observation_saturated: bool,
    raw_journal_reconciled: bool,
    raw_audit_ok: bool,
    journal_saturated: bool,
    post_only: bool,
    messages_path_only: bool,
    content_type_json_only: bool,
    content_encoding_absent: bool,
    authorization_absent: bool,
    x_api_key_redacted_only: bool,
    unknown_request_count: usize,
    strict_miss_count: usize,
    history_mismatch_count: usize,
    duplicate_request_count: usize,
    unused_scenario_count: usize,
    all_scenarios_consumed_once: bool,
    scenarios: Vec<AimockScenarioAudit>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AimockScenarioAudit {
    id: String,
    request_count: usize,
    method_ok: bool,
    path_ok: bool,
    content_type_json: bool,
    content_encoding_absent: bool,
    authorization_absent: bool,
    x_api_key_redacted: bool,
    status_ok: bool,
    model_ok: bool,
    stream_ok: bool,
    tools_ok: bool,
    expected_prior_history_count: usize,
    observed_prior_history_count: usize,
    total_input_count: usize,
    current_marker_position: i64,
    history_match: bool,
    tool_result_match: bool,
    read_round_match: bool,
    mismatch_code: usize,
    mismatch_index: usize,
}

struct AimockProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout_reader: Option<std::thread::JoinHandle<Result<Vec<u8>, ()>>>,
    stderr_reader: Option<std::thread::JoinHandle<Result<Vec<u8>, ()>>>,
    audit_path: PathBuf,
}

impl AimockProcess {
    fn start(
        control_root: &Path,
        process_home: &Path,
        scenario: &AimockScenarioDocument,
    ) -> Result<(Self, LocalAimockProvider), &'static str> {
        let metadata = std::fs::symlink_metadata(control_root)
            .map_err(|_| "cannot inspect aimock control directory")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("aimock control directory is not a no-follow directory");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if metadata.mode() & 0o7777 != 0o700 {
                return Err("aimock control directory does not have mode 0700");
            }
        }

        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("Tauri manifest has no repository parent")?;
        let harness = repository_root.join("scripts/live-claude-aimock-server.mjs");
        let harness_metadata = std::fs::symlink_metadata(&harness)
            .map_err(|_| "Claude aimock harness is unavailable")?;
        if !harness_metadata.is_file() || harness_metadata.file_type().is_symlink() {
            return Err("Claude aimock harness must be a no-follow regular file");
        }

        let scenario_path = control_root.join("aimock-scenario.json");
        let audit_path = control_root.join("aimock-audit.json");
        let scenario_bytes =
            serde_json::to_vec(scenario).map_err(|_| "cannot serialize Claude aimock scenario")?;
        if scenario_bytes.is_empty() || scenario_bytes.len() as u64 > MAX_CONTROL_BYTES {
            return Err("Claude aimock scenario exceeds its bound");
        }
        write_private_new(&scenario_path, &scenario_bytes);

        let mut command = closed_command("node");
        remove_sensitive_environment(&mut command);
        let mut child = command
            .arg(&harness)
            .arg("--scenario")
            .arg(&scenario_path)
            .arg("--audit")
            .arg(&audit_path)
            .current_dir(process_home)
            .env("HOME", process_home)
            .env("USERPROFILE", process_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "cannot start Claude aimock harness")?;
        let stdin = child.stdin.take().ok_or("aimock stdin is unavailable")?;
        let stdout = child.stdout.take().ok_or("aimock stdout is unavailable")?;
        let stderr = child.stderr.take().ok_or("aimock stderr is unavailable")?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let stdout_reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            let read_result = (&mut reader)
                .take(MAX_CONTROL_BYTES.saturating_add(1))
                .read_until(b'\n', &mut line);
            let result = match read_result {
                Ok(count)
                    if count > 0
                        && line.last() == Some(&b'\n')
                        && line.len() as u64 <= MAX_CONTROL_BYTES =>
                {
                    serde_json::from_slice::<AimockReadiness>(&line).map_err(|_| ())
                }
                _ => Err(()),
            };
            let _ = sender.send(result);
            let mut remainder = Vec::new();
            reader
                .take(MAX_CONTROL_BYTES.saturating_add(1))
                .read_to_end(&mut remainder)
                .map_err(|_| ())?;
            if remainder.len() as u64 > MAX_CONTROL_BYTES {
                return Err(());
            }
            Ok(remainder)
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr
                .take(MAX_CONTROL_BYTES.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|_| ())?;
            if bytes.len() as u64 > MAX_CONTROL_BYTES {
                return Err(());
            }
            Ok(bytes)
        });
        let readiness = match receiver.recv_timeout(AIMOCK_START_TIMEOUT) {
            Ok(Ok(readiness)) => readiness,
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("Claude aimock harness did not become ready");
            }
        };
        if readiness.kind != "mallard_claude_aimock_ready"
            || readiness.version != 1
            || readiness.port == 0
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err("Claude aimock harness emitted invalid readiness");
        }
        Ok((
            Self {
                child,
                stdin: Some(stdin),
                stdout_reader: Some(stdout_reader),
                stderr_reader: Some(stderr_reader),
                audit_path,
            },
            LocalAimockProvider::new(readiness.port),
        ))
    }

    fn finish(mut self, sensitive: &[Vec<u8>]) -> Result<(), &'static str> {
        self.stdin
            .as_mut()
            .ok_or("aimock stdin is closed")?
            .write_all(b"{\"command\":\"shutdown\",\"version\":1}\n")
            .and_then(|_| self.stdin.as_mut().expect("aimock stdin").flush())
            .map_err(|_| "cannot request Claude aimock shutdown")?;
        self.stdin.take();
        let deadline = Instant::now() + AIMOCK_STOP_TIMEOUT;
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return Err("Claude aimock harness did not stop cleanly");
                }
            }
        };
        let stdout_remainder = self
            .stdout_reader
            .take()
            .ok_or("aimock stdout reader is unavailable")?
            .join()
            .map_err(|_| "aimock stdout reader panicked")?
            .map_err(|_| "aimock stdout exceeded its bound")?;
        let stderr = self
            .stderr_reader
            .take()
            .ok_or("aimock stderr reader is unavailable")?
            .join()
            .map_err(|_| "aimock stderr reader panicked")?
            .map_err(|_| "aimock stderr exceeded its bound")?;
        assert_excludes(&stdout_remainder, sensitive, "Claude aimock stdout");
        assert_excludes(&stderr, sensitive, "Claude aimock stderr");
        if !status.success() {
            return Err("Claude aimock harness failed");
        }
        if !stdout_remainder.is_empty() || !stderr.is_empty() {
            return Err("Claude aimock harness emitted unexpected process output");
        }
        let audit = read_bounded(&self.audit_path, MAX_CONTROL_BYTES, "Claude aimock audit");
        assert_excludes(&audit, sensitive, "Claude aimock audit");
        validate_aimock_audit(&audit)
    }
}

impl Drop for AimockProcess {
    fn drop(&mut self) {
        self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

fn validate_aimock_audit(bytes: &[u8]) -> Result<(), &'static str> {
    let audit: AimockAudit =
        serde_json::from_slice(bytes).map_err(|_| "Claude aimock audit is malformed")?;
    if audit.raw_request_count != EXPECTED_AIMOCK_RAW_REQUESTS
        || audit.raw_expected_request_count != EXPECTED_AIMOCK_RAW_REQUESTS
        || audit.raw_model_request_count != EXPECTED_AIMOCK_REQUESTS
        || audit.raw_probe_request_count != EXPECTED_CURSOR_APPEND_PROCESSES
        || audit.raw_unexpected_request_count != 0
        || audit.raw_wrong_method_count != 0
        || audit.raw_wrong_path_count != 0
        || audit.raw_auth_rejection_count != 0
        || audit.raw_authorization_header_count != 0
        || audit.raw_alternate_credential_header_count != 0
        || audit.raw_upgrade_request_count != 0
    {
        eprintln!(
            "Claude aimock raw audit counters: requests={},expected={},models={},probes={},unexpected={},wrong_method={},wrong_path={},auth_rejection={},authorization={},alternate_credential={},upgrade={}",
            audit.raw_request_count,
            audit.raw_expected_request_count,
            audit.raw_model_request_count,
            audit.raw_probe_request_count,
            audit.raw_unexpected_request_count,
            audit.raw_wrong_method_count,
            audit.raw_wrong_path_count,
            audit.raw_auth_rejection_count,
            audit.raw_authorization_header_count,
            audit.raw_alternate_credential_header_count,
            audit.raw_upgrade_request_count,
        );
    }
    let aggregate_checks = [
        (audit.version == 1, "Claude aimock audit version failed"),
        (
            audit.expected_request_count == EXPECTED_AIMOCK_REQUESTS,
            "Claude aimock expected request count failed",
        ),
        (
            audit.expected_probe_request_count == EXPECTED_CURSOR_APPEND_PROCESSES,
            "Claude aimock expected probe request count failed",
        ),
        (
            audit.request_count == EXPECTED_AIMOCK_REQUESTS,
            "Claude aimock journal request count failed",
        ),
        (
            audit.unknown_request_count == 0,
            "Claude aimock unknown request audit failed",
        ),
        (
            audit.strict_miss_count == 0,
            "Claude aimock strict miss audit failed",
        ),
        (
            audit.history_mismatch_count == 0,
            "Claude aimock history audit failed",
        ),
        (
            audit.duplicate_request_count == 0,
            "Claude aimock duplicate audit failed",
        ),
        (
            audit.unused_scenario_count == 0,
            "Claude aimock unused scenario audit failed",
        ),
        (
            audit.all_scenarios_consumed_once,
            "Claude aimock scenario consumption audit failed",
        ),
        (audit.post_only, "Claude aimock method audit failed"),
        (audit.messages_path_only, "Claude aimock path audit failed"),
        (
            audit.authorization_absent,
            "Claude aimock Authorization audit failed",
        ),
        (
            audit.x_api_key_redacted_only,
            "Claude aimock redacted API-key audit failed",
        ),
        (
            audit.content_type_json_only,
            "Claude aimock content-type audit failed",
        ),
        (
            audit.content_encoding_absent,
            "Claude aimock content-encoding audit failed",
        ),
        (
            !audit.journal_saturated,
            "Claude aimock journal saturation audit failed",
        ),
        (
            audit.strict_mode,
            "Claude aimock strict configuration audit failed",
        ),
        (
            audit.loopback_only,
            "Claude aimock loopback configuration audit failed",
        ),
        (
            audit.recording_absent,
            "Claude aimock recording configuration audit failed",
        ),
        (
            audit.upstream_absent,
            "Claude aimock upstream configuration audit failed",
        ),
        (
            audit.inbound_auth_required,
            "Claude aimock inbound auth audit failed",
        ),
        (
            audit.fixed_placeholder_enforced,
            "Claude aimock fixed placeholder audit failed",
        ),
        (
            audit.raw_request_count == EXPECTED_AIMOCK_RAW_REQUESTS,
            "Claude aimock raw request count failed",
        ),
        (
            audit.raw_expected_request_count == EXPECTED_AIMOCK_RAW_REQUESTS,
            "Claude aimock raw expected request count failed",
        ),
        (
            audit.raw_model_request_count == EXPECTED_AIMOCK_REQUESTS,
            "Claude aimock raw model request count failed",
        ),
        (
            audit.raw_probe_request_count == EXPECTED_CURSOR_APPEND_PROCESSES,
            "Claude aimock raw probe request count failed",
        ),
        (
            audit.raw_unexpected_request_count == 0,
            "Claude aimock raw unexpected request audit failed",
        ),
        (
            audit.raw_wrong_method_count == 0,
            "Claude aimock raw method audit failed",
        ),
        (
            audit.raw_wrong_path_count == 0,
            "Claude aimock raw path audit failed",
        ),
        (
            audit.raw_auth_rejection_count == 0,
            "Claude aimock raw auth audit failed",
        ),
        (
            audit.raw_authorization_header_count == 0,
            "Claude aimock raw Authorization audit failed",
        ),
        (
            audit.raw_alternate_credential_header_count == 0,
            "Claude aimock raw alternate credential audit failed",
        ),
        (
            audit.raw_upgrade_request_count == 0,
            "Claude aimock raw upgrade audit failed",
        ),
        (
            !audit.raw_observation_saturated,
            "Claude aimock raw saturation audit failed",
        ),
        (
            audit.raw_journal_reconciled,
            "Claude aimock raw/journal reconciliation failed",
        ),
        (
            audit.raw_audit_ok,
            "Claude aimock raw boundary audit failed",
        ),
    ];
    if let Some((_, message)) = aggregate_checks.into_iter().find(|(passed, _)| !passed) {
        return Err(message);
    }
    if audit.scenarios.len() != EXPECTED_AIMOCK_REQUESTS {
        return Err("Claude aimock audit has the wrong scenario count");
    }
    let expected_ids = [
        "source_r1_read".to_string(),
        "source_r1_final".to_string(),
        "source_r2".to_string(),
        "source_r3".to_string(),
        "source_r4".to_string(),
        "source_r5".to_string(),
        "cursor_append".to_string(),
    ]
    .into_iter()
    .chain(
        (0..2).flat_map(|replica| (0..3).map(move |history| format!("verify_{replica}_{history}"))),
    )
    .collect::<BTreeSet<_>>();
    let mut ids = BTreeSet::new();
    for scenario in &audit.scenarios {
        if scenario.id.is_empty()
            || !ids.insert(scenario.id.clone())
            || scenario.request_count != 1
            || !scenario.history_match
            || !scenario.tool_result_match
            || !scenario.method_ok
            || !scenario.path_ok
            || !scenario.status_ok
            || !scenario.model_ok
            || !scenario.stream_ok
            || !scenario.authorization_absent
            || !scenario.x_api_key_redacted
            || !scenario.content_type_json
            || !scenario.content_encoding_absent
            || !scenario.tools_ok
            || !scenario.read_round_match
            || scenario.mismatch_code != 0
            || scenario.mismatch_index != 0
            || scenario.expected_prior_history_count != scenario.observed_prior_history_count
            || scenario.total_input_count <= scenario.observed_prior_history_count
            || scenario.current_marker_position < 0
            || usize::try_from(scenario.current_marker_position)
                .ok()
                .is_none_or(|position| position >= scenario.total_input_count)
        {
            return Err("Claude aimock per-scenario audit failed");
        }
    }
    if ids != expected_ids {
        return Err("Claude aimock audit scenario IDs changed");
    }
    Ok(())
}

fn stderr_synopsis(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    format!(
        "bytes={},lines={},error={},auth={},network={}",
        bytes.len(),
        text.lines().count(),
        text.contains("error"),
        text.contains("auth"),
        text.contains("network")
    )
}

fn stdout_synopsis(bytes: &[u8]) -> String {
    let parsed = serde_json::from_slice::<Value>(bytes).ok();
    let result = parsed
        .as_ref()
        .and_then(|value| value.get("result"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bounded_number_after = |prefix: &str| {
        result.find(prefix).and_then(|start| {
            let digits = result[start + prefix.len()..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .take(4)
                .collect::<Vec<_>>();
            (!digits.is_empty()).then(|| String::from_utf8_lossy(&digits).into_owned())
        })
    };
    format!(
        "bytes={},json={},type={},subtype={},is_error={},api={},connection={},rate_limit={},invalid={},s400={},s401={},s404={},s422={},s500={},s503={},s529={},fixture={},tool={},fetch={},socket={},eof={},abort={},http={},overloaded={},internal={},unexpected={},stream={},closed={},terminate={},retry={},unable={},code={},index={}",
        bytes.len(),
        parsed.is_some(),
        parsed
            .as_ref()
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("-"),
        parsed
            .as_ref()
            .and_then(|value| value.get("subtype"))
            .and_then(Value::as_str)
            .unwrap_or("-"),
        parsed
            .as_ref()
            .and_then(|value| value.get("is_error"))
            .and_then(Value::as_bool)
            .map_or("-", |value| if value { "true" } else { "false" }),
        result.contains("api"),
        result.contains("connect"),
        result.contains("rate limit"),
        result.contains("invalid"),
        result.contains("400"),
        result.contains("401"),
        result.contains("404"),
        result.contains("422"),
        result.contains("500"),
        result.contains("503"),
        result.contains("529"),
        result.contains("fixture"),
        result.contains("tool"),
        result.contains("fetch"),
        result.contains("socket"),
        result.contains("eof"),
        result.contains("abort"),
        result.contains("http"),
        result.contains("overload"),
        result.contains("internal"),
        result.contains("unexpected"),
        result.contains("stream"),
        result.contains("closed"),
        result.contains("terminat"),
        result.contains("retry"),
        result.contains("unable"),
        bounded_number_after("code=").unwrap_or_else(|| "-".to_string()),
        bounded_number_after("index=").unwrap_or_else(|| "-".to_string()),
    )
}

#[allow(clippy::too_many_arguments)]
fn run_claude(
    process_count: &mut usize,
    provider: &LocalAimockProvider,
    claude_home: &Path,
    process_home: &Path,
    project_root: &Path,
    session_id: &str,
    new_session: bool,
    prompt: &str,
    expected_response: &str,
) -> String {
    assert!(
        !prompt.is_empty()
            && prompt.len() <= MAX_PROMPT_BYTES
            && prompt
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'),
        "Claude prompt must be one bounded exact marker"
    );
    assert!(
        !expected_response.is_empty()
            && expected_response.len() <= 128
            && expected_response
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'),
        "Claude response marker is invalid"
    );
    assert!(
        !session_id.is_empty()
            && session_id.len() <= 128
            && session_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "Claude session ID is not a safe component"
    );
    let project_metadata =
        std::fs::symlink_metadata(project_root).expect("inspect Claude project directory");
    let home_metadata =
        std::fs::symlink_metadata(claude_home).expect("inspect isolated Claude home");
    assert!(
        project_metadata.is_dir()
            && home_metadata.is_dir()
            && !project_metadata.file_type().is_symlink()
            && !home_metadata.file_type().is_symlink(),
        "Claude roots must be no-follow directories"
    );

    let mut arguments = [
        "--bare",
        "--print",
        "--output-format",
        "json",
        "--model",
        MODEL,
        "--permission-mode",
        "dontAsk",
        "--tools=Read",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    if new_session {
        arguments.extend([OsString::from("--session-id"), OsString::from(session_id)]);
    } else {
        arguments.extend([OsString::from("--resume"), OsString::from(session_id)]);
    }
    arguments.push(OsString::from(prompt));
    assert!(
        arguments.len() <= MAX_COMMAND_ARGUMENTS
            && arguments
                .iter()
                .all(|argument| argument.as_encoded_bytes().len() <= MAX_COMMAND_ARGUMENT_BYTES),
        "Claude argv violates its safety bound"
    );
    assert!(
        arguments.iter().all(|argument| {
            !byte_slice_contains(argument.as_encoded_bytes(), PLACEHOLDER_API_KEY.as_bytes())
        }),
        "the local placeholder entered Claude argv"
    );

    let mut command = closed_command("claude");
    remove_sensitive_environment(&mut command);
    let mut child = command
        .args(arguments)
        .current_dir(project_root)
        .env("HOME", process_home)
        .env("USERPROFILE", process_home)
        .env("CLAUDE_CONFIG_DIR", claude_home)
        .env("ANTHROPIC_BASE_URL", &provider.base_url)
        .env("ANTHROPIC_API_KEY", PLACEHOLDER_API_KEY)
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env("DISABLE_TELEMETRY", "1")
        .env("DISABLE_ERROR_REPORTING", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start isolated Claude Code process");
    *process_count = process_count.saturating_add(1);
    let stdout = child.stdout.take().expect("capture Claude stdout");
    let stderr = child.stderr.take().expect("capture Claude stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take((MAX_PROCESS_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take((MAX_PROCESS_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + CLAUDE_TIMEOUT;
    loop {
        if child
            .try_wait()
            .expect("wait for isolated Claude process")
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            panic!("isolated Claude process exceeded its timeout");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let status = child.wait().expect("finish isolated Claude process");
    let stdout = stdout_reader
        .join()
        .expect("Claude stdout reader panicked")
        .expect("collect Claude stdout");
    let stderr = stderr_reader
        .join()
        .expect("Claude stderr reader panicked")
        .expect("collect Claude stderr");
    assert!(
        stdout.len() <= MAX_PROCESS_OUTPUT_BYTES && stderr.len() <= MAX_PROCESS_OUTPUT_BYTES,
        "Claude output exceeded its bound"
    );
    let sensitive = vec![
        PLACEHOLDER_API_KEY.as_bytes().to_vec(),
        INHERITED_SECRET_MARKER.as_bytes().to_vec(),
    ];
    assert_excludes(&stdout, &sensitive, "Claude stdout");
    assert_excludes(&stderr, &sensitive, "Claude stderr");
    let synopsis = stderr_synopsis(&stderr);
    let stdout_synopsis = stdout_synopsis(&stdout);
    assert!(
        status.success(),
        "isolated Claude process failed ({stdout_synopsis}; {synopsis})"
    );
    assert!(
        stderr.is_empty(),
        "isolated Claude process wrote stderr ({synopsis})"
    );
    let output: Value = serde_json::from_slice(&stdout)
        .unwrap_or_else(|_| panic!("Claude stdout was not one JSON result ({synopsis})"));
    assert_eq!(output.get("is_error").and_then(Value::as_bool), Some(false));
    assert_eq!(
        output.get("session_id").and_then(Value::as_str),
        Some(session_id),
        "Claude reported the wrong session identity"
    );
    assert_eq!(
        output.get("result").and_then(Value::as_str),
        Some(expected_response),
        "Claude returned the wrong deterministic marker"
    );
    let model_usage = output
        .get("modelUsage")
        .and_then(Value::as_object)
        .expect("Claude result has no model usage map");
    assert_eq!(
        model_usage
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([MODEL]),
        "Claude used an unexpected model or fallback"
    );
    wait_for_transcript_marker(claude_home, project_root, session_id, expected_response);
    session_id.to_string()
}

#[allow(clippy::too_many_arguments)]
fn run_live_tuzi_claude(
    process_count: &mut usize,
    provider: &LiveTuziConnection,
    claude_home: &Path,
    process_home: &Path,
    project_root: &Path,
    session_id: &str,
    new_session: bool,
    user_marker: &str,
    expected_response: &str,
) -> f64 {
    for (value, label) in [
        (user_marker, "live Claude user marker"),
        (expected_response, "live Claude response marker"),
    ] {
        assert!(
            !value.is_empty()
                && value.len() <= 128
                && value.bytes().all(|byte| {
                    byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                })
                && value.starts_with("MALLARD_CLAUDE_LIVE_"),
            "{label} is invalid"
        );
    }
    assert!(
        !session_id.is_empty()
            && session_id.len() <= 128
            && session_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "live Claude session ID is not a safe component"
    );
    for (path, label) in [
        (project_root, "live Claude project"),
        (claude_home, "live Claude home"),
        (process_home, "live Claude process home"),
    ] {
        let metadata =
            std::fs::symlink_metadata(path).unwrap_or_else(|_| panic!("cannot inspect {label}"));
        assert!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "{label} must be a no-follow directory"
        );
    }

    let prompt = format!("{user_marker} Reply exactly {expected_response}");
    let system_prompt =
        "Return only the exact MALLARD_CLAUDE_LIVE response marker requested by the user.";
    assert!(prompt.len() <= MAX_PROMPT_BYTES);
    let mut arguments = [
        "--bare",
        "--print",
        "--output-format",
        "json",
        "--model",
        LIVE_TUZI_MODEL,
        "--effort",
        "low",
        "--max-budget-usd",
        "0.05",
        "--max-turns",
        "1",
        "--permission-mode",
        "dontAsk",
        "--tools=",
        "--no-chrome",
        "--system-prompt",
        system_prompt,
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    if new_session {
        arguments.extend([OsString::from("--session-id"), OsString::from(session_id)]);
    } else {
        arguments.extend([OsString::from("--resume"), OsString::from(session_id)]);
    }
    arguments.push(OsString::from(prompt));
    assert!(
        arguments.len() <= MAX_COMMAND_ARGUMENTS
            && arguments
                .iter()
                .all(|argument| argument.as_encoded_bytes().len() <= MAX_COMMAND_ARGUMENT_BYTES),
        "live Claude argv violates its safety bound"
    );
    assert!(
        arguments.iter().all(|argument| {
            !byte_slice_contains(argument.as_encoded_bytes(), provider.api_key.as_bytes())
        }),
        "the live Claude credential entered argv"
    );

    let process_temp = process_home.join("tmp");
    secure_directory(&process_temp);
    let mut command = closed_command("claude");
    remove_sensitive_environment(&mut command);
    let mut child = command
        .args(arguments)
        .current_dir(project_root)
        .env("HOME", process_home)
        .env("USERPROFILE", process_home)
        .env("TMPDIR", &process_temp)
        .env("CLAUDE_CONFIG_DIR", claude_home)
        .env("ANTHROPIC_BASE_URL", &provider.base_url)
        .env("ANTHROPIC_API_KEY", &provider.api_key)
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env("CLAUDE_CODE_MAX_RETRIES", "0")
        .env("CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK", "1")
        .env("CLAUDE_CODE_MAX_OUTPUT_TOKENS", "1024")
        .env("API_TIMEOUT_MS", "180000")
        .env("DISABLE_TELEMETRY", "1")
        .env("DISABLE_ERROR_REPORTING", "1")
        .env("DISABLE_UPDATES", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start isolated paid Claude Code process");
    *process_count = process_count.saturating_add(1);
    let stdout = child.stdout.take().expect("capture paid Claude stdout");
    let stderr = child.stderr.take().expect("capture paid Claude stderr");
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take((MAX_PROCESS_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take((MAX_PROCESS_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + LIVE_TUZI_CLAUDE_TIMEOUT;
    loop {
        if child
            .try_wait()
            .expect("wait for paid Claude process")
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            panic!("paid Claude process exceeded its timeout");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let status = child.wait().expect("finish paid Claude process");
    let stdout = stdout_reader
        .join()
        .expect("paid Claude stdout reader panicked")
        .expect("collect paid Claude stdout");
    let stderr = stderr_reader
        .join()
        .expect("paid Claude stderr reader panicked")
        .expect("collect paid Claude stderr");
    assert!(
        stdout.len() <= MAX_PROCESS_OUTPUT_BYTES && stderr.len() <= MAX_PROCESS_OUTPUT_BYTES,
        "paid Claude output exceeded its bound"
    );
    let protected = provider.protected_needles();
    assert_excludes(&stdout, &protected, "paid Claude stdout");
    assert_excludes(&stderr, &protected, "paid Claude stderr");
    let synopsis = stderr_synopsis(&stderr);
    let stdout_synopsis = stdout_synopsis(&stdout);
    assert!(
        status.success(),
        "paid Claude process failed ({stdout_synopsis}; {synopsis})"
    );
    assert!(
        stderr.is_empty(),
        "paid Claude process wrote stderr ({synopsis})"
    );
    let output: Value = serde_json::from_slice(&stdout)
        .unwrap_or_else(|_| panic!("paid Claude stdout was not one JSON result ({synopsis})"));
    assert_eq!(output.get("type").and_then(Value::as_str), Some("result"));
    assert_eq!(
        output.get("subtype").and_then(Value::as_str),
        Some("success")
    );
    assert_eq!(output.get("is_error").and_then(Value::as_bool), Some(false));
    assert_eq!(output.get("num_turns").and_then(Value::as_u64), Some(1));
    assert_eq!(
        output.get("session_id").and_then(Value::as_str),
        Some(session_id),
        "paid Claude reported the wrong session identity"
    );
    assert_eq!(
        output.get("result").and_then(Value::as_str),
        Some(expected_response),
        "paid Claude returned the wrong deterministic marker"
    );
    let model_usage = output
        .get("modelUsage")
        .and_then(Value::as_object)
        .expect("paid Claude result has no model usage map");
    assert_eq!(
        model_usage
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([LIVE_TUZI_MODEL]),
        "paid Claude used an unexpected model or fallback"
    );
    let cost = output
        .get("total_cost_usd")
        .and_then(Value::as_f64)
        .expect("paid Claude result has no cost estimate");
    assert!(
        cost.is_finite() && (0.0..=MAX_LIVE_TUZI_COST_PER_TURN_USD).contains(&cost),
        "paid Claude exceeded its per-turn client budget"
    );
    wait_for_transcript_marker(claude_home, project_root, session_id, expected_response);
    cost
}

fn transcript_bucket_path(claude_home: &Path, project_root: &Path) -> PathBuf {
    let cwd = project_root
        .to_str()
        .expect("Claude live test requires UTF-8 temporary paths");
    claude_home
        .join("projects")
        .join(encode_claude_project_path(cwd))
}

fn transcript_path(claude_home: &Path, project_root: &Path, session_id: &str) -> PathBuf {
    transcript_bucket_path(claude_home, project_root).join(format!("{session_id}.jsonl"))
}

fn read_private_transcript(path: &Path, label: &str) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let metadata =
            std::fs::symlink_metadata(path).unwrap_or_else(|_| panic!("cannot inspect {label}"));
        let mode = metadata.mode() & 0o7777;
        assert_eq!(
            mode, 0o600,
            "{label} mode changed from the required 0600 (observed {mode:04o})"
        );
    }
    read_bounded(path, MAX_CONVERSATION_FILE_BYTES, label)
}

fn transcript_bytes(claude_home: &Path, project_root: &Path, session_id: &str) -> Vec<u8> {
    read_private_transcript(
        &transcript_path(claude_home, project_root, session_id),
        "Claude transcript",
    )
}

fn wait_for_transcript_marker(
    claude_home: &Path,
    project_root: &Path,
    session_id: &str,
    marker: &str,
) {
    let path = transcript_path(claude_home, project_root, session_id);
    let deadline = Instant::now() + TRANSCRIPT_SETTLE_TIMEOUT;
    loop {
        if std::fs::symlink_metadata(&path).is_ok() {
            let bytes = read_private_transcript(&path, "Claude transcript persistence check");
            let complete_records = if bytes.last() == Some(&b'\n') {
                bytes
                    .split(|byte| *byte == b'\n')
                    .filter(|line| !line.is_empty())
                    .map(serde_json::from_slice::<Value>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            } else {
                None
            };
            if complete_records.as_ref().is_some_and(|records| {
                records.iter().all(Value::is_object)
                    && records.iter().any(|record| {
                        record.get("type").and_then(Value::as_str) == Some("assistant")
                            && record
                                .get("message")
                                .and_then(|message| message.get("role"))
                                .and_then(Value::as_str)
                                == Some("assistant")
                            && record
                                .get("message")
                                .and_then(|message| message.get("content"))
                                .and_then(Value::as_array)
                                .is_some_and(|content| {
                                    content.iter().any(|block| {
                                        block.get("type").and_then(Value::as_str) == Some("text")
                                            && block.get("text").and_then(Value::as_str)
                                                == Some(marker)
                                    })
                                })
                    })
            }) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "Claude transcript did not durably record its deterministic response"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn root_level_transcript_stems(claude_home: &Path, project_root: &Path) -> BTreeSet<String> {
    let bucket = transcript_bucket_path(claude_home, project_root);
    let bucket_metadata =
        std::fs::symlink_metadata(&bucket).expect("inspect encoded Claude transcript bucket");
    assert!(
        bucket_metadata.is_dir() && !bucket_metadata.file_type().is_symlink(),
        "encoded Claude transcript bucket is not a no-follow directory"
    );
    let mut entry_count = 0_usize;
    let mut stems = BTreeSet::new();
    for entry in std::fs::read_dir(&bucket).expect("read encoded Claude transcript bucket") {
        let entry = entry.expect("read encoded Claude transcript entry");
        entry_count = entry_count.saturating_add(1);
        assert!(
            entry_count <= MAX_TREE_ENTRIES,
            "encoded Claude transcript bucket exceeds its entry bound"
        );
        let path = entry.path();
        if path.extension() != Some(std::ffi::OsStr::new("jsonl")) {
            continue;
        }
        let metadata =
            std::fs::symlink_metadata(&path).expect("inspect root-level Claude transcript entry");
        assert!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "root-level Claude JSONL is not a no-follow regular file"
        );
        let _ = read_private_transcript(&path, "root-level Claude transcript");
        let stem = path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .filter(|stem| !stem.is_empty())
            .expect("root-level Claude transcript has no UTF-8 stem")
            .to_string();
        assert!(
            stems.insert(stem),
            "encoded Claude transcript bucket contains duplicate stems"
        );
    }
    stems
}

fn fixed_activity_before_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock predates the Unix epoch")
        .as_secs()
        .saturating_add(60 * 60)
}

/// The two independent reads the Activity page performs — the session list
/// and the storage comparison — joined into one value with the exact shape
/// the retired single workspace command produced, so every assertion over
/// it is unchanged.
#[derive(Serialize, Clone)]
struct ActivityWorkspaceView {
    activity: ProjectActivity,
    #[serde(skip_serializing_if = "Option::is_none")]
    comparison: Option<ThreadSyncComparison>,
    #[serde(skip_serializing_if = "Option::is_none")]
    comparison_error: Option<String>,
}

fn activity_workspace(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    storage_id: &StorageId,
    before_time: u64,
) -> ActivityWorkspaceView {
    let activity = redacted(
        service.get_project_activity(
            project_id,
            Some(storage_id),
            None,
            Some(before_time),
            Some(90),
        ),
        "cannot load the Claude Activity list",
    );
    let (comparison, comparison_error) = match service.compare_threads(project_id, storage_id) {
        Ok(comparison) => (Some(comparison), None),
        Err(error) => (None, Some(error)),
    };
    ActivityWorkspaceView {
        activity,
        comparison,
        comparison_error,
    }
}

fn wait_for_activity_workspace(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    storage_id: &StorageId,
    before_time: u64,
    mut ready: impl FnMut(&ActivityWorkspaceView) -> bool,
) -> ActivityWorkspaceView {
    let deadline = Instant::now() + TRANSCRIPT_SETTLE_TIMEOUT;
    loop {
        let workspace = activity_workspace(service, project_id, storage_id, before_time);
        if ready(&workspace) {
            return workspace;
        }
        assert!(
            Instant::now() < deadline,
            "Claude Activity did not converge before its bounded deadline"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn remove_volatile_workspace_fields(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                remove_volatile_workspace_fields(item);
            }
        }
        Value::Object(object) => {
            object.remove("compared_at");
            object.remove("is_active");
            for item in object.values_mut() {
                remove_volatile_workspace_fields(item);
            }
        }
        _ => {}
    }
}

fn stable_activity_workspace(workspace: &ActivityWorkspaceView) -> Value {
    let mut value = serde_json::to_value(workspace).expect("serialize Claude Activity workspace");
    remove_volatile_workspace_fields(&mut value);
    value
}

fn assert_activity_counts(
    session: &ProjectSessionSummary,
    user_round_count: u64,
    agent_message_count: u64,
    tool_call_count: u64,
) {
    assert_eq!(session.user_round_count, user_round_count);
    assert_eq!(session.agent_message_count, agent_message_count);
    assert_eq!(session.tool_call_count, tool_call_count);
    assert!(
        session.metrics_complete,
        "Claude Activity metrics are partial"
    );
    assert_activity_token_totals(session);
}

fn assert_activity_token_totals(session: &ProjectSessionSummary) {
    let total_input_tokens = session
        .total_input_tokens
        .expect("Claude Activity total input tokens are unavailable");
    let total_output_tokens = session
        .total_output_tokens
        .expect("Claude Activity total output tokens are unavailable");
    let total_tokens = session
        .total_tokens
        .expect("Claude Activity total tokens are unavailable");
    assert_eq!(
        total_tokens,
        total_input_tokens.saturating_add(total_output_tokens),
        "Claude Activity total tokens do not match input plus output"
    );
    assert!(
        total_input_tokens > 0 && total_output_tokens > 0,
        "Claude aimock transcript has no token usage"
    );
}

fn assert_activity_matches_session_detail(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    session: &ProjectSessionSummary,
) {
    let resource_id =
        ResourceId::parse(session.resource_id.clone()).expect("parse Claude Activity resource ID");
    let detail = service
        .get_project_session_detail(project_id, &resource_id)
        .expect("cannot load Claude Session Detail for Activity count comparison");
    let prompts = detail
        .event_counts
        .prompts
        .value
        .expect("Claude Session Detail prompt count is unavailable");
    let responses = detail
        .event_counts
        .responses
        .value
        .expect("Claude Session Detail response count is unavailable");
    let tool_calls = detail
        .event_counts
        .tool_calls
        .value
        .expect("Claude Session Detail tool-call count is unavailable");

    assert_eq!(session.user_round_count, prompts);
    assert_eq!(session.agent_message_count, responses);
    assert_eq!(session.tool_call_count, tool_calls);
    assert_activity_token_totals(session);
    assert!(tool_calls > 0, "Claude aimock transcript has no tool call");
}

fn marker_history_activity_counts(history: &[HistoryMarker]) -> (u64, u64, u64) {
    let user_markers = history
        .iter()
        .filter(|marker| marker.role == HistoryRole::User)
        .count() as u64;
    let assistant_markers = history
        .iter()
        .filter(|marker| marker.role == HistoryRole::Assistant)
        .count() as u64;
    // Every lineage in this fixture inherits R1's one tool-use assistant
    // record and one user-role tool-result record. Those records remain one
    // tool call, but neither is a human prompt or terminal assistant reply.
    (user_markers, assistant_markers, 1)
}

fn write_foreign_cwd_decoy(source: &[u8], destination: &Path, foreign_project_root: &Path) {
    let mut rewritten = Vec::with_capacity(source.len());
    for raw in source
        .split(|byte| *byte == b'\n')
        .filter(|raw| !raw.is_empty())
    {
        let mut value: Value = serde_json::from_slice(raw).expect("decoy source line is JSON");
        let object = value
            .as_object_mut()
            .expect("decoy source line is an object");
        if object.contains_key("sessionId") {
            object.insert(
                "sessionId".to_string(),
                Value::String(FOREIGN_DECOY_SESSION_ID.to_string()),
            );
        }
        if object.contains_key("cwd") {
            object.insert(
                "cwd".to_string(),
                Value::String(foreign_project_root.to_string_lossy().into_owned()),
            );
        }
        rewritten.extend(serde_json::to_vec(&value).expect("encode foreign-cwd decoy line"));
        rewritten.push(b'\n');
    }
    write_private_new(destination, &rewritten);
}

fn assert_identity_namespaces_distinct(local: &str, provider: &str, cloud: &str, label: &str) {
    assert_eq!(
        [local, provider, cloud]
            .into_iter()
            .collect::<BTreeSet<_>>()
            .len(),
        3,
        "{label} reused an identity across local/provider/cloud namespaces"
    );
}

fn assert_uuid_v4(value: &str, label: &str) {
    let bytes = value.as_bytes();
    let hyphens = [8, 13, 18, 23];
    let shape_ok = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            if hyphens.contains(&index) {
                *byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
            }
        });
    assert!(shape_ok, "{label} is not a lowercase RFC 4122 UUID");
    assert_eq!(bytes[14], b'4', "{label} is not UUID version 4");
    assert!(
        matches!(bytes[19], b'8' | b'9' | b'a' | b'b'),
        "{label} does not have the RFC 4122 variant"
    );
}

fn tree_digest_inventory(root: &Path) -> BTreeMap<PathBuf, [u8; 32]> {
    let mut inventory = BTreeMap::new();
    let mut entry_count = 0_usize;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = redacted(entry, "cannot inventory isolated tree");
        entry_count = entry_count.saturating_add(1);
        assert!(
            entry_count <= MAX_TREE_ENTRIES,
            "isolated tree exceeds its entry bound"
        );
        assert!(
            !entry.file_type().is_symlink(),
            "isolated tree contains a symlink"
        );
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("tree entry remains below root")
            .to_path_buf();
        let bytes = read_bounded(entry.path(), MAX_STORAGE_OBJECT_BYTES, "isolated tree file");
        inventory.insert(relative, Sha256::digest(bytes).into());
    }
    inventory
}

fn assert_private_tree(root: &Path, sensitive: &[Vec<u8>], label: &str) {
    let root_metadata =
        std::fs::symlink_metadata(root).unwrap_or_else(|_| panic!("cannot inspect {label}"));
    assert!(
        root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
        "{label} is not a no-follow directory"
    );
    let mut count = 0_usize;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = redacted(entry, "cannot inspect isolated privacy tree");
        count = count.saturating_add(1);
        assert!(count <= MAX_TREE_ENTRIES, "{label} exceeds its entry bound");
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("privacy entry remains below root");
        assert_excludes(
            relative.as_os_str().as_encoded_bytes(),
            sensitive,
            "isolated tree path",
        );
        assert!(
            !entry.file_type().is_symlink(),
            "{label} contains a symlink"
        );
        if entry.file_type().is_dir() {
            continue;
        }
        assert!(
            entry.file_type().is_file(),
            "{label} contains a special file"
        );
        assert!(
            entry.file_name() != std::ffi::OsStr::new(".credentials.json"),
            "{label} contains Claude credentials"
        );
        let bytes = read_bounded(entry.path(), MAX_STORAGE_OBJECT_BYTES, "privacy tree file");
        assert_excludes(&bytes, sensitive, "isolated tree file");
    }
}

fn local_storage(storage_root: &Path) -> StorageConfig {
    local_storage_config(
        StorageId::parse("storage-live-claude-aimock").expect("static storage ID"),
        "Live Claude aimock storage",
        storage_root,
    )
}

fn live_tuzi_storage(storage_root: &Path) -> StorageConfig {
    local_storage_config(
        StorageId::parse("storage-live-claude-tuzi").expect("static storage ID"),
        "Live Claude Tuzi storage",
        storage_root,
    )
}

fn finish_setup(
    state: &LocalStateRepository,
    service: &ProjectSyncService<'_>,
    project_root: &Path,
    claude_home: &Path,
    storage: StorageConfig,
    repository: DraftRepositoryChoice,
) -> ProjectDetail {
    let storage_id = storage.id.clone();
    let mut config = redacted(state.load_config(), "cannot load storage configuration");
    if !config.storages.iter().any(|stored| stored.id == storage_id) {
        config.storages.push(storage);
        redacted(
            state.save_config(config),
            "cannot save storage configuration",
        );
    }
    let mut draft: ProjectSetupDraft = redacted(
        service.create_setup_draft(project_root),
        "cannot create Claude-only setup draft",
    )
    .draft;
    // New drafts may select a default Codex profile. This target deliberately
    // proves that Claude alone is sufficient, so remove every implicit choice.
    draft.profiles.clear();
    draft.profiles.insert(
        claude_key(),
        DraftProfileSelection::Pending {
            path: claude_home.to_string_lossy().into_owned(),
            display_name: String::new(),
        },
    );
    draft.storage = Some(DraftStorageSelection { storage_id });
    draft.repository = repository;
    let draft = redacted(
        service.update_setup_draft(draft),
        "cannot save Claude-only setup choices",
    );
    let inspection = redacted(
        service.inspect_setup_draft(&draft.draft_id),
        "cannot inspect Claude-only setup",
    );
    assert!(
        inspection.can_finalize,
        "Claude-only setup is not finalizable"
    );
    redacted(
        service.finalize_project_setup(&draft.draft_id, draft.revision),
        "cannot finalize Claude-only setup",
    )
}

fn approved_pull_actions(plan: &RestorePlan) -> Vec<ActionId> {
    plan.actions
        .iter()
        .filter(|action| !matches!(action.kind, RestoreActionKind::Manual { .. }))
        .map(|action| action.action_id.clone())
        .chain(
            plan.conversation_review
                .iter()
                .filter_map(|row| row.action_id.clone()),
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
        "cannot plan reviewed Claude Pull",
    );
    let result = redacted(
        service.apply_bundle_restore(&plan, approved_pull_actions(&plan)),
        "cannot apply reviewed Claude Pull",
    );
    assert!(result.success && result.failed_actions.is_empty());
    assert_eq!(result.reviewed_base_advanced, Some(true));
    (plan, result)
}

fn push(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> ProjectOperationResult {
    let review = redacted(
        service.prepare_push_review(project_id, storage_id),
        "cannot prepare reviewed Claude Push",
    );
    assert!(matches!(
        review.destination_status,
        PushDestinationStatus::Ready { .. }
    ));
    redacted(
        service.push_reviewed(
            project_id,
            storage_id,
            PushApproval::recommended(&review, storage_id),
        ),
        "cannot apply reviewed Claude Push",
    )
}

fn clear_storage_for_replica(
    repository: &LocalStateRepository,
    service: &ProjectSyncService<'_>,
    storage_id: &StorageId,
) -> StorageDataClearResult {
    let expected_config_revision = redacted(
        repository.load_config(),
        "cannot load config before storage clear",
    )
    .revision;
    redacted(
        service.clear_storage_data(storage_id, expected_config_revision),
        "cannot clear storage through the production service",
    )
}

fn conversation_binding(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    thread_id: &ThreadId,
) -> LocalConversationBinding {
    redacted(
        repository.load_conversation_bindings(),
        "cannot load Claude conversation bindings",
    )
    .bindings
    .into_iter()
    .find(|binding| binding.local_project_id == *project_id && binding.thread_id == *thread_id)
    .expect("Claude logical thread has no local binding")
}

fn logical_thread_id(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    provider_thread_id: &str,
) -> ThreadId {
    redacted(
        repository.load_conversation_bindings(),
        "cannot load Claude conversation bindings",
    )
    .bindings
    .into_iter()
    .find(|binding| {
        binding.local_project_id == *project_id
            && binding.provider_thread_id.as_deref() == Some(provider_thread_id)
    })
    .map(|binding| binding.thread_id)
    .expect("Mallard did not bind the real Claude session")
}

fn provider_thread_id(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    thread_id: &ThreadId,
) -> String {
    conversation_binding(repository, project_id, thread_id)
        .provider_thread_id
        .expect("Claude logical thread is not resumable")
}

fn cloud_thread_id_of(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    thread_id: &ThreadId,
) -> CloudThreadId {
    conversation_binding(repository, project_id, thread_id)
        .cloud_thread_id
        .expect("Claude logical thread has no cloud mapping")
}

fn local_thread_of_cloud(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    cloud_thread_id: &CloudThreadId,
) -> ThreadId {
    redacted(
        repository.load_conversation_bindings(),
        "cannot load Claude conversation bindings",
    )
    .bindings
    .into_iter()
    .find(|binding| {
        binding.local_project_id == *project_id
            && binding.cloud_thread_id.as_ref() == Some(cloud_thread_id)
    })
    .map(|binding| binding.thread_id)
    .expect("Claude cloud thread has no local registration")
}

fn assert_keep_both_offered(plan: &RestorePlan, thread_id: &ThreadId) {
    let actions = plan
        .conversation_review
        .iter()
        .filter(|row| {
            row.thread_id.as_ref() == Some(thread_id)
                && row.action == ConversationReviewAction::KeepBoth
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actions.len(),
        1,
        "divergent Claude Pull must offer one Keep Both"
    );
}

struct KeepBothOutcome {
    preserved_thread_id: ThreadId,
    remote_provider_thread_id: String,
}

fn assert_keep_both_result(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    tracking_thread_id: &ThreadId,
    cloud_thread_id: &CloudThreadId,
    old_provider_thread_id: &str,
    result: &RestoreResult,
) -> KeepBothOutcome {
    assert_eq!(result.preserved_threads.len(), 1);
    let preserved = &result.preserved_threads[0];
    assert_eq!(preserved.original_thread_id, *tracking_thread_id);
    let preserved_thread_id = preserved.preserved_thread_id.clone();
    let tracking = conversation_binding(repository, project_id, tracking_thread_id);
    assert_eq!(tracking.cloud_thread_id.as_ref(), Some(cloud_thread_id));
    let remote_provider_thread_id = tracking
        .provider_thread_id
        .expect("Keep Both did not mint the remote Claude provider ID");
    assert_ne!(remote_provider_thread_id, old_provider_thread_id);
    assert_eq!(
        provider_thread_id(repository, project_id, &preserved_thread_id),
        old_provider_thread_id,
        "Keep Both did not retain the local Claude provider ID"
    );
    let preserved_binding = conversation_binding(repository, project_id, &preserved_thread_id);
    assert_eq!(preserved_binding.cloud_thread_id, None);
    let provenance = preserved_binding
        .preserved
        .expect("Keep Both did not persist preservation provenance");
    assert_eq!(provenance.preserved_from, *cloud_thread_id);
    assert!(provenance.ordinal > 0);
    KeepBothOutcome {
        preserved_thread_id,
        remote_provider_thread_id,
    }
}

fn assert_divergent_push_gate(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    storage_id: &StorageId,
    cloud_thread_id: &CloudThreadId,
    replica: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let comparison = redacted(
            service.compare_threads(project_id, storage_id),
            "cannot compare divergent Claude threads",
        );
        if comparison.entries.iter().any(|entry| {
            entry.cloud_thread_id.as_ref() == Some(cloud_thread_id)
                && entry.state == ThreadSyncState::Diverged
        }) {
            break;
        }
        if Instant::now() >= deadline {
            panic!("{replica} did not observe the expected Claude divergence");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let review = redacted(
        service.prepare_push_review(project_id, storage_id),
        "cannot prepare divergent Claude Push",
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
        .expect("divergent Claude Push has no comparison");
    let entry = comparison
        .entries
        .iter()
        .find(|entry| {
            entry.cloud_thread_id.as_ref() == Some(cloud_thread_id)
                && entry.state == ThreadSyncState::Diverged
        })
        .expect("divergent Claude Push lacks its cloud row");
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
        "stale Claude replica unexpectedly pushed"
    );
}

fn assert_fixed_point(comparison: &ThreadSyncComparison, revision: u64) {
    assert_eq!(comparison.revision, Some(revision));
    assert_eq!(comparison.base_revision, Some(revision));
    assert!(
        comparison
            .entries
            .iter()
            .all(|entry| entry.state == ThreadSyncState::Synced),
        "Claude comparison is not at a fixed point: {:?}",
        comparison
            .entries
            .iter()
            .map(|entry| (&entry.cloud_thread_id, entry.state))
            .collect::<Vec<_>>()
    );
    assert_eq!(comparison.counts.diverged, 0);
    assert_eq!(comparison.counts.local, 0);
    assert_eq!(comparison.counts.storage, 0);
}

#[derive(Debug)]
struct ReplicaTopology {
    by_cloud: BTreeMap<String, (String, String)>,
}

#[allow(clippy::too_many_arguments)]
fn assert_exact_replica_topology(
    repository: &LocalStateRepository,
    service: &ProjectSyncService<'_>,
    binding: &ProjectBinding,
    project_id: &LocalProjectId,
    storage_id: &StorageId,
    claude_home: &Path,
    project_root: &Path,
    expected_cloud_ids: &BTreeSet<String>,
    expected_revision: u64,
    label: &str,
) -> ReplicaTopology {
    assert_eq!(
        binding.profile_ids.len(),
        1,
        "{label} is not bound exclusively to Claude"
    );
    let profile_id = binding
        .profile_ids
        .get(&claude_key())
        .expect("Claude-only binding has no Claude profile");
    let document = redacted(
        repository.load_conversation_bindings(),
        "cannot load final Claude conversation bindings",
    );
    let rows = document
        .bindings
        .into_iter()
        .filter(|row| row.local_project_id == *project_id)
        .collect::<Vec<_>>();
    assert_eq!(
        rows.len(),
        expected_cloud_ids.len(),
        "{label} has extra or missing Claude conversation bindings"
    );

    let mut by_cloud = BTreeMap::new();
    let mut local_ids = BTreeSet::new();
    let mut provider_ids = BTreeSet::new();
    for row in rows {
        assert_eq!(
            row.provider_profile_id, *profile_id,
            "{label} row references an unexpected provider profile"
        );
        assert_eq!(row.host_id, binding.host_id, "{label} row has wrong host");
        assert_eq!(
            row.replica_id, binding.replica_id,
            "{label} row has wrong replica"
        );
        assert_eq!(
            row.materialization_state,
            ConversationMaterializationState::Resumable,
            "{label} contains a non-resumable Claude row"
        );
        let cloud_id = row
            .cloud_thread_id
            .expect("final Claude binding has no cloud identity")
            .as_str()
            .to_string();
        let local_id = row.thread_id.as_str().to_string();
        let provider_id = row
            .provider_thread_id
            .expect("final resumable Claude binding has no provider identity");
        assert_identity_namespaces_distinct(&local_id, &provider_id, &cloud_id, label);
        if provider_id != SOURCE_SESSION_ID && provider_id != LIVE_TUZI_SOURCE_SESSION_ID {
            assert_uuid_v4(&provider_id, "production-minted Claude provider ID");
        }
        assert!(
            local_ids.insert(local_id.clone()),
            "{label} reused a local thread identity"
        );
        assert!(
            provider_ids.insert(provider_id.clone()),
            "{label} reused a provider session identity"
        );
        assert!(
            by_cloud.insert(cloud_id, (local_id, provider_id)).is_none(),
            "{label} has duplicate cloud conversation identities"
        );
    }
    assert_eq!(
        by_cloud.keys().cloned().collect::<BTreeSet<_>>(),
        *expected_cloud_ids,
        "{label} has the wrong cloud conversation set"
    );
    assert_eq!(
        local_ids.len(),
        expected_cloud_ids.len(),
        "{label} local identity count changed"
    );
    assert_eq!(
        provider_ids.len(),
        expected_cloud_ids.len(),
        "{label} provider identity count changed"
    );
    assert_eq!(
        root_level_transcript_stems(claude_home, project_root),
        provider_ids,
        "{label} Claude bucket does not exactly match its resumable bindings"
    );

    let comparison = redacted(
        service.compare_threads(project_id, storage_id),
        "cannot compare final Claude topology",
    );
    assert_fixed_point(&comparison, expected_revision);
    assert_eq!(
        comparison.entries.len(),
        expected_cloud_ids.len(),
        "{label} comparison has extra or missing conversation rows"
    );
    assert_eq!(
        comparison.counts.synced,
        expected_cloud_ids.len(),
        "{label} comparison has the wrong synced count"
    );
    assert_eq!(comparison.counts.unavailable, 0);
    assert_eq!(comparison.counts.unknown, 0);
    assert!(comparison.provider_errors.is_empty());
    let mut comparison_clouds = BTreeSet::new();
    for entry in &comparison.entries {
        assert_eq!(entry.provider, claude_key());
        assert!(entry.local_present && entry.storage_present);
        assert!(entry.compared_against_storage);
        assert_eq!(entry.state, ThreadSyncState::Synced);
        let cloud_id = entry
            .cloud_thread_id
            .as_ref()
            .expect("final comparison row has no cloud identity")
            .as_str();
        assert!(comparison_clouds.insert(cloud_id.to_string()));
        let (_, provider_id) = by_cloud
            .get(cloud_id)
            .expect("comparison contains an unexpected cloud conversation");
        assert_eq!(
            entry.provider_thread_id.as_deref(),
            Some(provider_id.as_str())
        );
    }
    assert_eq!(
        comparison_clouds, *expected_cloud_ids,
        "{label} comparison has the wrong cloud conversation set"
    );
    ReplicaTopology { by_cloud }
}

fn assert_cross_replica_topology(
    replica_a: &ReplicaTopology,
    replica_b: &ReplicaTopology,
    expected_cloud_ids: &BTreeSet<String>,
) {
    assert_eq!(
        replica_a.by_cloud.keys().cloned().collect::<BTreeSet<_>>(),
        *expected_cloud_ids
    );
    assert_eq!(
        replica_b.by_cloud.keys().cloned().collect::<BTreeSet<_>>(),
        *expected_cloud_ids
    );
    let local_a = replica_a
        .by_cloud
        .values()
        .map(|(local, _)| local.clone())
        .collect::<BTreeSet<_>>();
    let local_b = replica_b
        .by_cloud
        .values()
        .map(|(local, _)| local.clone())
        .collect::<BTreeSet<_>>();
    let provider_a = replica_a
        .by_cloud
        .values()
        .map(|(_, provider)| provider.clone())
        .collect::<BTreeSet<_>>();
    let provider_b = replica_b
        .by_cloud
        .values()
        .map(|(_, provider)| provider.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(local_a.len(), expected_cloud_ids.len());
    assert_eq!(local_b.len(), expected_cloud_ids.len());
    assert_eq!(provider_a.len(), expected_cloud_ids.len());
    assert_eq!(provider_b.len(), expected_cloud_ids.len());
    assert!(
        local_a.is_disjoint(&local_b),
        "replicas copied local thread identities"
    );
    assert!(
        provider_a.is_disjoint(&provider_b),
        "replicas copied provider session identities"
    );
    let all_local = local_a
        .iter()
        .chain(&local_b)
        .cloned()
        .collect::<BTreeSet<_>>();
    let all_provider = provider_a
        .iter()
        .chain(&provider_b)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(all_local.len(), expected_cloud_ids.len() * 2);
    assert_eq!(all_provider.len(), expected_cloud_ids.len() * 2);
    assert!(all_local.is_disjoint(&all_provider));
    assert!(all_local.is_disjoint(expected_cloud_ids));
    assert!(all_provider.is_disjoint(expected_cloud_ids));
    for cloud_id in expected_cloud_ids {
        let (local_a, provider_a) = &replica_a.by_cloud[cloud_id];
        let (local_b, provider_b) = &replica_b.by_cloud[cloud_id];
        assert_ne!(local_a, local_b, "cloud lineage reused a local identity");
        assert_ne!(
            provider_a, provider_b,
            "cloud lineage reused a provider identity"
        );
    }
}

fn assert_final_activity_workspace(
    workspace: &ActivityWorkspaceView,
    topology: &ReplicaTopology,
    histories: &[(CloudThreadId, Vec<HistoryMarker>)],
    primary_cloud: &CloudThreadId,
    expected_revision: u64,
    label: &str,
) {
    assert!(
        workspace.comparison_error.is_none(),
        "{label} Activity comparison failed"
    );
    assert_fixed_point(
        workspace
            .comparison
            .as_ref()
            .expect("final Activity workspace has no comparison"),
        expected_revision,
    );
    assert_eq!(
        workspace.activity.sessions.len(),
        topology.by_cloud.len(),
        "{label} Activity has extra or missing sessions"
    );
    assert!(workspace.activity.unmapped.is_empty());
    assert!(workspace.activity.warnings.is_empty());

    let expected_resources = topology
        .by_cloud
        .values()
        .map(|(local_id, _)| {
            let local_id = ThreadId::parse(local_id).expect("parse final local thread ID");
            mallard_domain::conversation_resource_id(&claude_key(), &local_id)
                .expect("derive final Activity resource ID")
                .to_string()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        workspace
            .activity
            .sessions
            .iter()
            .map(|session| session.resource_id.clone())
            .collect::<BTreeSet<_>>(),
        expected_resources,
        "{label} Activity roster differs from conversation bindings"
    );

    let mut preservation_ordinals = BTreeSet::new();
    for (cloud_id, (local_id, provider_id)) in &topology.by_cloud {
        let session = workspace
            .activity
            .sessions
            .iter()
            .find(|session| session.logical_thread_id.as_deref() == Some(local_id.as_str()))
            .expect("final Activity lacks a bound conversation");
        assert_eq!(session.provider, claude_key());
        assert_eq!(
            session.provider_session_id.as_deref(),
            Some(provider_id.as_str())
        );
        assert_eq!(session.source, SessionSourceState::LocalAndRemote);
        let history = histories
            .iter()
            .find(|(history_cloud, _)| history_cloud.as_str() == cloud_id)
            .map(|(_, history)| history.as_slice())
            .expect("final Activity cloud identity has no history oracle");
        let (users, agents, tools) = marker_history_activity_counts(history);
        assert_activity_counts(session, users, agents, tools);

        if cloud_id == primary_cloud.as_str() {
            assert!(
                session.preserved.is_none(),
                "{label} primary Activity row is incorrectly preserved"
            );
        } else {
            let preserved = session
                .preserved
                .as_ref()
                .expect("preserved Activity row lacks provenance");
            assert_eq!(preserved.preserved_from, *primary_cloud);
            assert!(preserved.ordinal > 0);
            assert!(
                preservation_ordinals.insert(preserved.ordinal),
                "{label} reused a preservation ordinal"
            );
        }
    }
    assert_eq!(
        preservation_ordinals.len(),
        topology.by_cloud.len().saturating_sub(1),
        "{label} Activity has the wrong preserved-row count"
    );
}

fn assert_pull_is_no_op(
    service: &ProjectSyncService<'_>,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    binding: &ProjectBinding,
    expected_revision: u64,
) {
    let plan = redacted(
        service.plan_bundle_restore(storage_id, bundle_id, binding),
        "cannot plan fixed-point Claude Pull",
    );
    assert_eq!(
        plan.revision, expected_revision,
        "fixed-point Pull planned a new revision"
    );
    assert!(plan.actions.is_empty(), "fixed-point Pull has file actions");
    assert!(
        plan.conversation_actions.is_empty(),
        "fixed-point Pull has conversation actions"
    );
    let result = redacted(
        service.apply_bundle_restore(&plan, Vec::new()),
        "cannot apply fixed-point reviewed Claude Pull",
    );
    assert!(result.success, "fixed-point reviewed Pull did not succeed");
    assert!(
        result.failed_actions.is_empty(),
        "fixed-point reviewed Pull reported failures"
    );
    assert!(
        result.applied_action_ids.is_empty(),
        "fixed-point reviewed Pull applied an action"
    );
    assert_eq!(
        result.reviewed_base_advanced,
        Some(true),
        "fixed-point reviewed Pull did not confirm its accepted base"
    );
    assert!(result.preserved_threads.is_empty());
}

fn marker_from_text(role: HistoryRole, text: &str) -> Option<HistoryMarker> {
    let prefix = text
        .bytes()
        .take_while(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
        .count();
    if prefix == 0 || prefix > 128 {
        return None;
    }
    let value = &text[..prefix];
    value
        .starts_with("MALLARD_CLAUDE_")
        .then(|| marker(role, value))
}

fn timeline_history(
    adapter: &dyn SessionAdapter,
    transcript: &Path,
    project_root: &Path,
    provider_thread_id: &str,
) -> Vec<HistoryMarker> {
    let context = SessionTimelineContext {
        resource_id: "claude:conversation:live-history-oracle",
        provider_session_id: provider_thread_id,
        project_root,
        project_root_aliases: &[],
        provider_home: project_root,
        child: None,
    };
    let index = adapter
        .build_timeline_index(transcript, &context)
        .expect("production Claude timeline could not index the transcript");
    assert!(!index.truncated, "production Claude timeline was truncated");
    assert!(index.events.len() <= MAX_TIMELINE_EVENTS);
    let included = BTreeSet::from([
        SessionTimelineEventKind::Prompt,
        SessionTimelineEventKind::Response,
    ]);
    let mut start_index = None;
    let mut history = Vec::new();
    let mut pages = 0_usize;
    loop {
        pages = pages.saturating_add(1);
        assert!(pages <= MAX_TIMELINE_EVENTS);
        let page = adapter
            .read_timeline_page(
                transcript,
                &index,
                &SessionTimelineQuery {
                    start_index,
                    limit: 2,
                    included_event_kinds: included.clone(),
                    included_tool_categories: BTreeSet::new(),
                    turn_id: None,
                },
                &context,
            )
            .expect("production Claude timeline page failed");
        for event in page.events {
            assert!(matches!(
                event.quality,
                SessionCountQuality::Exact | SessionCountQuality::Inferred
            ));
            let text = event.text.expect("Claude Prompt/Response has no text");
            let role = match event.kind {
                SessionTimelineEventKind::Prompt => HistoryRole::User,
                SessionTimelineEventKind::Response => HistoryRole::Assistant,
                _ => panic!("Claude history page returned an unrequested event"),
            };
            if let Some(marker) = marker_from_text(role, &text) {
                history.push(marker);
            }
        }
        let Some(next) = page.next_index else {
            break;
        };
        assert!(next > start_index.unwrap_or(0) && next <= index.events.len());
        start_index = Some(next);
    }
    history
}

fn claude_record_shape_synopsis(bytes: &[u8]) -> String {
    let mut shapes = BTreeMap::<String, usize>::new();
    for raw in bytes
        .split(|byte| *byte == b'\n')
        .filter(|raw| !raw.is_empty())
        .take(256)
    {
        let Ok(record) = serde_json::from_slice::<Value>(raw) else {
            *shapes.entry("malformed".to_string()).or_default() += 1;
            continue;
        };
        let record_type = record.get("type").and_then(Value::as_str).unwrap_or("-");
        let role = record
            .pointer("/message/role")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let blocks = record
            .pointer("/message/content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block.get("type").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("+")
            })
            .unwrap_or_else(|| "-".to_string());
        *shapes
            .entry(format!("{record_type}/{role}/{blocks}"))
            .or_default() += 1;
    }
    shapes
        .into_iter()
        .map(|(shape, count)| format!("{shape}={count}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn assert_transcript_structure_and_history(
    claude_home: &Path,
    project_root: &Path,
    provider_thread_id: &str,
    expected_history: &[HistoryMarker],
    expected_durable_records: usize,
    read_marker_path: &Path,
) {
    let path = transcript_path(claude_home, project_root, provider_thread_id);
    let bytes = read_private_transcript(&path, "Claude history transcript");
    let cwd = project_root.to_string_lossy();
    for raw in bytes
        .split(|byte| *byte == b'\n')
        .filter(|raw| !raw.is_empty())
    {
        let value: Value = serde_json::from_slice(raw).expect("Claude transcript line is JSON");
        let object = value
            .as_object()
            .expect("Claude transcript line is an object");
        if let Some(session_id) = object.get("sessionId").and_then(Value::as_str) {
            assert_eq!(
                session_id, provider_thread_id,
                "Claude transcript retained a foreign structural session ID"
            );
        }
        if let Some(observed_cwd) = object.get("cwd").and_then(Value::as_str) {
            assert_eq!(
                observed_cwd,
                cwd.as_ref(),
                "Claude transcript retained a foreign structural cwd"
            );
        }
    }
    let adapter = ClaudeAdapter::new();
    let analysis = adapter.analyze(&bytes);
    assert!(
        analysis.is_resumable(),
        "production Claude analysis rejected a bounded native transcript: records={},lineage={:?}",
        analysis.record_count,
        analysis.lineage
    );
    assert_eq!(
        analysis.source_provider_session_id.as_deref(),
        Some(provider_thread_id)
    );
    assert_eq!(analysis.source_cwd.as_deref(), Some(cwd.as_ref()));
    let durable_records = analysis
        .lineage
        .chain()
        .expect("Claude lineage is complete")
        .len();
    // Claude Code 2.1.266 can link the final tool-round response either
    // through the tool-result row or directly through the tool-use row. The
    // latter leaves exactly that result off the active ancestry; the timeline
    // checks below still require the tool result and its marker.
    assert!(
        durable_records == expected_durable_records
            || durable_records.saturating_add(1) == expected_durable_records,
        "Claude active-lineage record count changed: expected {} or {}, got {}; shapes: {}",
        expected_durable_records.saturating_sub(1),
        expected_durable_records,
        durable_records,
        claude_record_shape_synopsis(&bytes),
    );
    assert_eq!(
        timeline_history(&adapter, &path, project_root, provider_thread_id),
        expected_history,
        "production Claude history parser returned the wrong marker sequence"
    );
    let context = SessionTimelineContext {
        resource_id: "claude:conversation:live-tool-oracle",
        provider_session_id: provider_thread_id,
        project_root,
        project_root_aliases: &[],
        provider_home: project_root,
        child: None,
    };
    let index = adapter
        .build_timeline_index(&path, &context)
        .expect("production Claude tool timeline failed");
    let tools = index
        .events
        .iter()
        .filter(|event| event.kind == SessionTimelineEventKind::ToolCall)
        .collect::<Vec<_>>();
    assert_eq!(tools.len(), 1, "R1 Read round did not survive exactly once");
    let tool = tools[0]
        .tool
        .as_ref()
        .expect("Claude tool lacks index data");
    assert_eq!(tool.provider_call_id, READ_TOOL_CALL_ID);
    assert_eq!(tool.native_name, "Read");
    assert_eq!(tool.category, SessionToolCategory::Read);
    assert_eq!(tool.status, SessionToolCallStatus::Completed);
    assert!(tool.affected_paths.is_empty());
    let detail = adapter
        .read_event_detail(&path, &index, &tools[0].event_id)
        .expect("cannot expand production Claude Read detail");
    let source_project_root = read_marker_path
        .parent()
        .expect("Read marker path has no project parent");
    let expected_argument_path =
        if project_root == source_project_root || provider_thread_id == FOREIGN_DECOY_SESSION_ID {
            read_marker_path.to_path_buf()
        } else {
            project_root.join(
                read_marker_path
                    .file_name()
                    .expect("Read marker path has no file name"),
            )
        };
    let arguments = detail
        .arguments
        .as_deref()
        .expect("Claude Read detail has no arguments");
    let arguments_value = serde_json::from_str::<Value>(arguments)
        .expect("Claude Read detail arguments are not JSON");
    let actual_argument_path = arguments_value
        .get("file_path")
        .and_then(Value::as_str)
        .map(Path::new)
        .expect("Claude Read detail has no file_path");
    assert!(
        actual_argument_path == expected_argument_path,
        "Claude Read detail has the wrong local path presentation"
    );
    if expected_argument_path != read_marker_path {
        assert!(
            !actual_argument_path.starts_with(source_project_root),
            "Claude Read detail retained the source project path after relocation"
        );
    }
    assert!(
        detail
            .result
            .as_deref()
            .is_some_and(|result| result.contains(READ_FILE_MARKER)),
        "Claude Read detail lost its result marker"
    );
}

fn session_detail_history(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    resource_id: &ResourceId,
    provider_thread_id: &str,
    expected: &[HistoryMarker],
    project_root: &Path,
    read_marker_path: &Path,
) {
    let header = service
        .get_project_session_detail(project_id, resource_id)
        .expect("cannot load Claude Session Detail header");
    assert_eq!(header.resource_id, resource_id.as_str());
    assert_eq!(header.provider, claude_key());
    assert_eq!(header.provider_session_id, provider_thread_id);
    assert_eq!(
        header.event_counts.prompts.value,
        Some(
            expected
                .iter()
                .filter(|item| item.role == HistoryRole::User)
                .count() as u64
        )
    );
    assert_eq!(
        header.event_counts.responses.value,
        Some(
            expected
                .iter()
                .filter(|item| item.role == HistoryRole::Assistant)
                .count() as u64
        )
    );
    assert_eq!(header.event_counts.tool_calls.value, Some(1));
    assert_eq!(header.event_counts.reads.value, Some(1));
    for count in [
        &header.event_counts.prompts,
        &header.event_counts.responses,
        &header.event_counts.tool_calls,
        &header.event_counts.reads,
    ] {
        assert_eq!(
            count.quality,
            SessionCountQuality::Exact,
            "pinned Claude transcript did not yield exact Session Detail counts"
        );
    }

    let included = BTreeSet::from([
        SessionTimelineEventKind::Prompt,
        SessionTimelineEventKind::Response,
    ]);
    let mut cursor = None;
    let mut history = Vec::new();
    let mut cursors = BTreeSet::new();
    loop {
        let page = service
            .get_project_session_timeline_page(
                project_id,
                resource_id,
                ProjectSessionTimelineRequest {
                    snapshot_id: header.snapshot_id.clone(),
                    cursor: cursor.clone(),
                    limit: Some(2),
                    included_event_kinds: included.clone(),
                    included_tool_categories: BTreeSet::new(),
                    turn_id: None,
                },
            )
            .expect("cannot page Claude Session Detail history");
        for event in page.events {
            let role = match event.kind {
                SessionTimelineEventKind::Prompt => HistoryRole::User,
                SessionTimelineEventKind::Response => HistoryRole::Assistant,
                _ => panic!("Session Detail returned an unrequested history event"),
            };
            if let Some(text) = event.text {
                if let Some(marker) = marker_from_text(role, &text) {
                    history.push(marker);
                }
            }
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        assert!(cursors.insert(next.clone()) && cursors.len() <= MAX_TIMELINE_EVENTS);
        cursor = Some(next);
    }
    assert_eq!(
        history, expected,
        "Session Detail returned the wrong Claude history"
    );

    let tools = service
        .get_project_session_timeline_page(
            project_id,
            resource_id,
            ProjectSessionTimelineRequest {
                snapshot_id: header.snapshot_id.clone(),
                cursor: None,
                limit: Some(2),
                included_event_kinds: BTreeSet::from([SessionTimelineEventKind::ToolCall]),
                included_tool_categories: BTreeSet::from([SessionToolCategory::Read]),
                turn_id: None,
            },
        )
        .expect("cannot load Claude Session Detail tool page");
    assert_eq!(tools.events.len(), 1);
    assert!(tools.next_cursor.is_none());
    let event = &tools.events[0];
    let preview = event
        .tool
        .as_ref()
        .expect("Session Detail tool lacks preview");
    assert_eq!(preview.provider_call_id, READ_TOOL_CALL_ID);
    assert_eq!(preview.native_name, "Read");
    assert_eq!(preview.category, SessionToolCategory::Read);
    assert_eq!(preview.status, SessionToolCallStatus::Completed);
    assert!(preview.affected_paths.is_empty());
    let detail = service
        .get_project_session_event_detail(
            project_id,
            resource_id,
            &header.snapshot_id,
            &event.event_id,
        )
        .expect("cannot expand Claude Session Detail Read event");
    let arguments = detail
        .arguments
        .as_deref()
        .expect("Session Detail Read event has no arguments");
    let source_project_root = read_marker_path
        .parent()
        .expect("Read marker path has no project parent");
    let arguments_value = serde_json::from_str::<Value>(arguments)
        .expect("Session Detail Read arguments are not JSON");
    assert_eq!(
        arguments_value.get("file_path").and_then(Value::as_str),
        Some("./read-marker.txt"),
        "Session Detail Read arguments have the wrong local path presentation"
    );
    assert!(
        !arguments.contains(project_root.to_string_lossy().as_ref()),
        "Session Detail exposed the selected replica's absolute project root"
    );
    assert!(
        !arguments.contains(source_project_root.to_string_lossy().as_ref()),
        "Session Detail exposed the source replica's absolute project root"
    );
    assert!(detail
        .result
        .as_deref()
        .is_some_and(|result| result.contains(READ_FILE_MARKER)));
}

fn assert_live_text_transcript(
    claude_home: &Path,
    project_root: &Path,
    provider_thread_id: &str,
    expected_history: &[HistoryMarker],
    expected_durable_records: usize,
) {
    let path = transcript_path(claude_home, project_root, provider_thread_id);
    let bytes = read_private_transcript(&path, "paid Claude history transcript");
    let cwd = project_root.to_string_lossy();
    let mut assistant_models = BTreeSet::new();
    for raw in bytes
        .split(|byte| *byte == b'\n')
        .filter(|raw| !raw.is_empty())
    {
        let value: Value =
            serde_json::from_slice(raw).expect("paid Claude transcript line is JSON");
        let object = value
            .as_object()
            .expect("paid Claude transcript line is an object");
        if let Some(session_id) = object.get("sessionId").and_then(Value::as_str) {
            assert_eq!(
                session_id, provider_thread_id,
                "paid Claude transcript retained a foreign structural session ID"
            );
        }
        if let Some(observed_cwd) = object.get("cwd").and_then(Value::as_str) {
            assert_eq!(
                observed_cwd,
                cwd.as_ref(),
                "paid Claude transcript retained a foreign structural cwd"
            );
        }
        if object.get("type").and_then(Value::as_str) == Some("assistant") {
            if let Some(observed_model) = object
                .get("message")
                .and_then(|message| message.get("model"))
                .and_then(Value::as_str)
            {
                assistant_models.insert(observed_model.to_string());
            }
        }
    }
    assert_eq!(
        assistant_models,
        BTreeSet::from([LIVE_TUZI_MODEL.to_string()]),
        "paid Claude transcript contains an unexpected model or fallback"
    );

    let adapter = ClaudeAdapter::new();
    let analysis = adapter.analyze(&bytes);
    assert!(
        analysis.is_resumable(),
        "production Claude analysis rejected the paid transcript: records={},lineage={:?}",
        analysis.record_count,
        analysis.lineage
    );
    assert_eq!(
        analysis.source_provider_session_id.as_deref(),
        Some(provider_thread_id)
    );
    assert_eq!(analysis.source_cwd.as_deref(), Some(cwd.as_ref()));
    assert_eq!(
        analysis
            .lineage
            .chain()
            .expect("paid Claude lineage is complete")
            .len(),
        expected_durable_records
    );
    assert_eq!(
        timeline_history(&adapter, &path, project_root, provider_thread_id),
        expected_history,
        "production Claude history parser returned the wrong paid marker sequence"
    );
    let context = SessionTimelineContext {
        resource_id: "claude:conversation:live-tuzi-text-oracle",
        provider_session_id: provider_thread_id,
        project_root,
        project_root_aliases: &[],
        provider_home: project_root,
        child: None,
    };
    let index = adapter
        .build_timeline_index(&path, &context)
        .expect("production Claude timeline could not index the paid transcript");
    assert!(
        index
            .events
            .iter()
            .all(|event| event.kind != SessionTimelineEventKind::ToolCall),
        "the zero-tool paid Claude flow recorded a tool call"
    );
}

fn session_detail_text_history(
    service: &ProjectSyncService<'_>,
    project_id: &LocalProjectId,
    resource_id: &ResourceId,
    provider_thread_id: &str,
    expected: &[HistoryMarker],
) {
    let header = service
        .get_project_session_detail(project_id, resource_id)
        .expect("cannot load paid Claude Session Detail header");
    assert_eq!(header.resource_id, resource_id.as_str());
    assert_eq!(header.provider, claude_key());
    assert_eq!(header.provider_session_id, provider_thread_id);
    assert_eq!(
        header.event_counts.prompts.value,
        Some(
            expected
                .iter()
                .filter(|item| item.role == HistoryRole::User)
                .count() as u64
        )
    );
    assert_eq!(
        header.event_counts.responses.value,
        Some(
            expected
                .iter()
                .filter(|item| item.role == HistoryRole::Assistant)
                .count() as u64
        )
    );
    assert_eq!(header.event_counts.tool_calls.value, Some(0));
    assert_eq!(header.event_counts.reads.value, Some(0));
    for count in [
        &header.event_counts.prompts,
        &header.event_counts.responses,
        &header.event_counts.tool_calls,
        &header.event_counts.reads,
    ] {
        assert_eq!(
            count.quality,
            SessionCountQuality::Exact,
            "paid Claude transcript did not yield exact Session Detail counts"
        );
    }

    let included = BTreeSet::from([
        SessionTimelineEventKind::Prompt,
        SessionTimelineEventKind::Response,
    ]);
    let mut cursor = None;
    let mut history = Vec::new();
    let mut cursors = BTreeSet::new();
    loop {
        let page = service
            .get_project_session_timeline_page(
                project_id,
                resource_id,
                ProjectSessionTimelineRequest {
                    snapshot_id: header.snapshot_id.clone(),
                    cursor: cursor.clone(),
                    limit: Some(2),
                    included_event_kinds: included.clone(),
                    included_tool_categories: BTreeSet::new(),
                    turn_id: None,
                },
            )
            .expect("cannot page paid Claude Session Detail history");
        for event in page.events {
            let role = match event.kind {
                SessionTimelineEventKind::Prompt => HistoryRole::User,
                SessionTimelineEventKind::Response => HistoryRole::Assistant,
                _ => panic!("paid Session Detail returned an unrequested history event"),
            };
            if let Some(text) = event.text {
                if let Some(marker) = marker_from_text(role, &text) {
                    history.push(marker);
                }
            }
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        assert!(cursors.insert(next.clone()) && cursors.len() <= MAX_TIMELINE_EVENTS);
        cursor = Some(next);
    }
    assert_eq!(
        history, expected,
        "Session Detail returned the wrong paid Claude history"
    );

    let tools = service
        .get_project_session_timeline_page(
            project_id,
            resource_id,
            ProjectSessionTimelineRequest {
                snapshot_id: header.snapshot_id,
                cursor: None,
                limit: Some(8),
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
        .expect("cannot load paid Claude Session Detail tool page");
    assert!(tools.events.is_empty() && tools.next_cursor.is_none());
}

fn assert_portable_state_is_safe(
    service: &ProjectSyncService<'_>,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    storage_root: &Path,
    sensitive: &[Vec<u8>],
    path_needles: &[Vec<u8>],
    machine_identity_needles: &[Vec<u8>],
) {
    let snapshot = redacted(
        service.inspect_bundle(storage_id, bundle_id),
        "cannot inspect published Claude bundle",
    );
    let manifest =
        serde_json::to_vec(&snapshot.manifest).expect("serialize Claude bundle manifest");
    assert_excludes(&manifest, sensitive, "portable Claude manifest");
    assert_excludes(&manifest, path_needles, "portable Claude manifest");
    assert_excludes(
        &manifest,
        machine_identity_needles,
        "portable Claude manifest",
    );
    let mut entry_count = 0_usize;
    for entry in WalkDir::new(storage_root).follow_links(false) {
        let entry = redacted(entry, "cannot inspect Claude local-folder storage");
        entry_count = entry_count.saturating_add(1);
        assert!(
            entry_count <= MAX_TREE_ENTRIES,
            "Claude local-folder storage exceeds its entry bound"
        );
        assert!(
            !entry.file_type().is_symlink(),
            "storage contains a symlink"
        );
        let relative = entry
            .path()
            .strip_prefix(storage_root)
            .expect("storage entry remains below storage root");
        assert_excludes(
            relative.as_os_str().as_encoded_bytes(),
            sensitive,
            "storage object key",
        );
        assert_excludes(
            relative.as_os_str().as_encoded_bytes(),
            path_needles,
            "storage object key",
        );
        assert_excludes(
            relative.as_os_str().as_encoded_bytes(),
            machine_identity_needles,
            "storage object key",
        );
        if entry.file_type().is_dir() {
            continue;
        }
        assert!(
            entry.file_type().is_file(),
            "storage contains a special file"
        );
        let bytes = read_bounded(
            entry.path(),
            MAX_STORAGE_OBJECT_BYTES,
            "Claude storage object",
        );
        // Transcript payloads intentionally carry the structural source cwd.
        // Credentials, however, may never enter any portable object body.
        assert_excludes(&bytes, sensitive, "Claude storage object");
    }
}

fn install_snapshot_transcript(
    source_home: &Path,
    snapshot_home: &Path,
    project_root: &Path,
    provider_thread_id: &str,
) {
    secure_directory(snapshot_home);
    let source = transcript_path(source_home, project_root, provider_thread_id);
    let destination = transcript_path(snapshot_home, project_root, provider_thread_id);
    secure_directory(
        destination
            .parent()
            .expect("snapshot transcript has a parent directory"),
    );
    let bytes = read_private_transcript(&source, "snapshot source transcript");
    write_private_new(&destination, &bytes);
}

#[allow(clippy::too_many_arguments)]
fn verify_final_histories(
    process_count: &mut usize,
    provider: &LocalAimockProvider,
    sensitive: &[Vec<u8>],
    read_marker_path: &Path,
    replicas: [(
        &LocalStateRepository,
        &ProjectSyncService<'_>,
        &LocalProjectId,
        &Path,
        &Path,
    ); 2],
    histories: &[(CloudThreadId, Vec<HistoryMarker>)],
) {
    for (replica_index, (repository, service, project_id, claude_home, project_root)) in
        replicas.into_iter().enumerate()
    {
        let authoritative_before = tree_digest_inventory(claude_home);
        for (history_index, (cloud_thread_id, expected)) in histories.iter().enumerate() {
            let local_thread_id = local_thread_of_cloud(repository, project_id, cloud_thread_id);
            let provider_thread_id = provider_thread_id(repository, project_id, &local_thread_id);
            let expected_durable_records = expected.len().saturating_add(2);
            assert_transcript_structure_and_history(
                claude_home,
                project_root,
                &provider_thread_id,
                expected,
                expected_durable_records,
                read_marker_path,
            );
            let resource_id =
                mallard_domain::conversation_resource_id(&claude_key(), &local_thread_id)
                    .expect("derive final Claude Session Detail resource ID");
            session_detail_history(
                service,
                project_id,
                &resource_id,
                &provider_thread_id,
                expected,
                project_root,
                read_marker_path,
            );

            let snapshot = tempfile::tempdir().expect("create Claude reopen snapshot root");
            let snapshot_home = snapshot.path().join("claude-home");
            let snapshot_process_home = snapshot.path().join("process-home");
            secure_directory(&snapshot_process_home);
            install_snapshot_transcript(
                claude_home,
                &snapshot_home,
                project_root,
                &provider_thread_id,
            );
            run_claude(
                process_count,
                provider,
                &snapshot_home,
                &snapshot_process_home,
                project_root,
                &provider_thread_id,
                false,
                &format!("MALLARD_CLAUDE_USER_VERIFY_{replica_index}_{history_index}"),
                FINAL_REOPEN_ACK,
            );
            let _ = read_private_transcript(
                &transcript_path(&snapshot_home, project_root, &provider_thread_id),
                "Claude reopened snapshot transcript",
            );
            assert_private_tree(&snapshot_home, sensitive, "Claude reopen snapshot");
            assert_private_tree(
                &snapshot_process_home,
                sensitive,
                "Claude reopen process home",
            );
            assert_eq!(
                tree_digest_inventory(claude_home),
                authoritative_before,
                "snapshot reopen mutated an authoritative Claude home"
            );
        }
        assert_eq!(tree_digest_inventory(claude_home), authoritative_before);
    }
}

fn lineage_of(bytes: &[u8]) -> mallard_infrastructure::conversation_codec::CodecLineage {
    let analysis = ClaudeAdapter::new().analyze(bytes);
    assert!(analysis.is_resumable());
    analysis.lineage
}

/// The lineage expected after local bytes are path-encoded, redacted, and
/// decoded for a destination binding. The destination provider identity is
/// restamped after decoding, as it is in the production Pull path.
fn relocated_lineage_of(
    bytes: &[u8],
    source_root: &Path,
    destination_root: &Path,
    process_home: &Path,
    source_provider_thread_id: &str,
    destination_provider_thread_id: &str,
) -> mallard_infrastructure::conversation_codec::CodecLineage {
    let text = std::str::from_utf8(bytes).expect("Claude transcript is UTF-8");
    let source_mapper = SessionPathMapper::new(
        source_root.to_str().expect("source project path is UTF-8"),
        None,
        process_home.to_str(),
    )
    .expect("build source path mapper");
    let mut encoded = String::with_capacity(text.len());
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            encoded.push('\n');
        }
        if line.trim().is_empty() {
            encoded.push_str(line);
        } else {
            encoded.push_str(
                &source_mapper
                    .encode_jsonl_line_gated(line)
                    .expect("path-encode Claude transcript line"),
            );
        }
    }
    let (redacted, _changed_lines) = mallard_infrastructure::credential_redaction::shared()
        .redact_jsonl_text(&encoded)
        .expect("transcript redacts cleanly");
    let destination_mapper = SessionPathMapper::new(
        destination_root
            .to_str()
            .expect("destination project path is UTF-8"),
        None,
        process_home.to_str(),
    )
    .expect("build destination path mapper");
    let mut relocated = String::with_capacity(redacted.len());
    for (index, line) in redacted.split('\n').enumerate() {
        if index > 0 {
            relocated.push('\n');
        }
        if line.trim().is_empty() {
            relocated.push_str(line);
            continue;
        }
        let mut value = destination_mapper
            .decode_jsonl_line_verified(line)
            .expect("path-decode Claude transcript line");
        if value.get("sessionId").and_then(Value::as_str) == Some(source_provider_thread_id) {
            value["sessionId"] = Value::String(destination_provider_thread_id.to_string());
        }
        relocated.push_str(
            &serde_json::to_string(&value).expect("serialize relocated Claude transcript line"),
        );
    }
    lineage_of(relocated.as_bytes())
}

#[test]
fn live_tuzi_provider_document_is_strict_and_uses_the_site_root() {
    for url in ["https://api.tu-zi.com/", "https://api.tu-zi.com/v1"] {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "_type": "newapi_channel_conn",
            "key": "unit-test-live-claude-key",
            "url": url,
        }))
        .unwrap();
        let connection = parse_live_tuzi_connection(&bytes).unwrap();
        assert_eq!(connection.api_key, "unit-test-live-claude-key");
        assert_eq!(connection.base_url, "https://api.tu-zi.com");
        assert_eq!(
            connection.protected_needles(),
            vec![
                b"unit-test-live-claude-key".to_vec(),
                INHERITED_SECRET_MARKER.as_bytes().to_vec(),
            ]
        );
    }
}

#[test]
fn live_tuzi_provider_document_rejects_legacy_or_unsafe_shapes() {
    let invalid = [
        serde_json::json!({
            "_type": "legacy",
            "key": "key",
            "url": "https://api.tu-zi.com/",
        }),
        serde_json::json!({
            "_type": "newapi_channel_conn",
            "key": " key ",
            "url": "https://api.tu-zi.com/",
        }),
        serde_json::json!({
            "_type": "newapi_channel_conn",
            "key": "key",
            "url": "http://api.tu-zi.com/",
        }),
        serde_json::json!({
            "_type": "newapi_channel_conn",
            "key": "key",
            "url": "https://api.tu-zi.com.evil.example/",
        }),
        serde_json::json!({
            "_type": "newapi_channel_conn",
            "key": "key",
            "url": "https://user@api.tu-zi.com/",
        }),
        serde_json::json!({
            "_type": "newapi_channel_conn",
            "key": "key",
            "url": "https://api.tu-zi.com:444/",
        }),
        serde_json::json!({
            "_type": "newapi_channel_conn",
            "key": "key",
            "url": "https://api.tu-zi.com/v1/messages",
        }),
        serde_json::json!({
            "_type": "newapi_channel_conn",
            "key": "key",
            "url": "https://api.tu-zi.com/?route=other",
        }),
        serde_json::json!({
            "_type": "newapi_channel_conn",
            "key": "key",
            "url": "https://api.tu-zi.com/#fragment",
        }),
        serde_json::json!({
            "_type": "newapi_channel_conn",
            "key": "key",
            "url": "https://api.tu-zi.com/",
            "legacy": true,
        }),
    ];
    for document in invalid {
        assert!(parse_live_tuzi_connection(&serde_json::to_vec(&document).unwrap()).is_err());
    }
    assert!(parse_live_tuzi_connection(b"{}").is_err());
    assert!(parse_live_tuzi_connection(b"not-json").is_err());
}

#[test]
#[ignore = "runs a heavyweight deterministic local Claude Code/aimock integration flow; set MALLARD_RUN_LIVE_CLAUDE_AIMOCK_E2E=1"]
fn live_claude_aimock_setup_push_pull_divergence() {
    if std::env::var(LIVE_TEST_GATE).ok().as_deref() != Some("1") {
        return;
    }

    let root = tempfile::tempdir().expect("create isolated Claude live-test root");
    let process_home = root.path().join("process-home");
    let claude_home_a = root.path().join("claude-a");
    let claude_home_b = root.path().join("claude-b");
    let project_a_input = root.path().join("project-a");
    let foreign_project_input = root.path().join("project.a");
    let project_b_input = root.path().join("project-b");
    let storage_root = root.path().join("storage");
    let machine_a_home = root.path().join("machine-a");
    let machine_b_home = root.path().join("machine-b");
    let aimock_control = root.path().join("aimock-control");
    for directory in [
        &process_home,
        &claude_home_a,
        &claude_home_b,
        &project_a_input,
        &foreign_project_input,
        &project_b_input,
        &storage_root,
        &machine_a_home,
        &machine_b_home,
        &aimock_control,
    ] {
        secure_directory(directory);
    }
    let project_a =
        std::fs::canonicalize(&project_a_input).expect("canonicalize replica-A Claude project");
    let foreign_project = std::fs::canonicalize(&foreign_project_input)
        .expect("canonicalize foreign colliding Claude project");
    let project_b =
        std::fs::canonicalize(&project_b_input).expect("canonicalize replica-B Claude project");
    let canonical_root =
        std::fs::canonicalize(root.path()).expect("canonicalize Claude live-test root");
    assert_eq!(std::fs::canonicalize(&project_a).unwrap(), project_a);
    assert_eq!(std::fs::canonicalize(&project_b).unwrap(), project_b);
    assert_ne!(project_a, foreign_project);
    assert_eq!(
        encode_claude_project_path(project_a.to_string_lossy().as_ref()),
        encode_claude_project_path(foreign_project.to_string_lossy().as_ref()),
        "the decoy fixture no longer exercises a lossy Claude bucket collision"
    );

    assert_supported_claude_version(&process_home, EXPECTED_AIMOCK_CLAUDE_VERSION);
    let read_marker_path = project_a.join("read-marker.txt");
    // The Read tool ingests this file verbatim, so its planted (fake)
    // credentials ride the transcript's tool-result record into every
    // push — the live proof that no detectable secret reaches the store
    // and that typed placeholders arrive in its place.
    write_private_new(
        &read_marker_path,
        format!(
            "{READ_FILE_MARKER}\nDB_PASSWORD={PLANTED_LIVE_PASSWORD}\n\
             TOKEN={PLANTED_LIVE_ANTHROPIC_KEY}\n"
        )
        .as_bytes(),
    );
    let scenario = aimock_scenario_document(&read_marker_path);
    let _home_guard = EnvironmentGuard::set("HOME", &process_home);
    let _userprofile_guard = EnvironmentGuard::set("USERPROFILE", &process_home);
    let (aimock, provider) = AimockProcess::start(&aimock_control, &process_home, &scenario)
        .expect("start deterministic Claude aimock server");

    let repository_a = redacted(
        LocalStateRepository::from_home_dir(&machine_a_home),
        "cannot create replica-A Mallard repository",
    );
    let repository_b = redacted(
        LocalStateRepository::from_home_dir(&machine_b_home),
        "cannot create replica-B Mallard repository",
    );
    let service_a = ProjectSyncService::new(&repository_a);
    let service_b = ProjectSyncService::new(&repository_b);
    let storage = local_storage(&storage_root);
    let storage_id = storage.id.clone();
    let sensitive = vec![
        PLACEHOLDER_API_KEY.as_bytes().to_vec(),
        INHERITED_SECRET_MARKER.as_bytes().to_vec(),
    ];
    // Store scans additionally forbid the planted transcript credentials;
    // the process-level scans keep the base set (the aimock audit
    // legitimately records the tool result that carries them).
    let mut store_sensitive = sensitive.clone();
    store_sensitive.push(PLANTED_LIVE_ANTHROPIC_KEY.as_bytes().to_vec());
    store_sensitive.push(PLANTED_LIVE_PASSWORD.as_bytes().to_vec());
    let mut path_needles = vec![root.path().to_string_lossy().as_bytes().to_vec()];
    let canonical_needle = canonical_root.to_string_lossy().as_bytes().to_vec();
    if !path_needles.contains(&canonical_needle) {
        path_needles.push(canonical_needle);
    }
    let mut process_count = 0_usize;
    let activity_before_time = fixed_activity_before_time();

    // Setup publishes the registration-only revision 1. Content revision 2:
    // replica A creates and publishes a real tool-bearing Claude session.
    let first = finish_setup(
        &repository_a,
        &service_a,
        &project_a,
        &claude_home_a,
        storage.clone(),
        DraftRepositoryChoice::New,
    );
    let binding_a = first
        .binding
        .clone()
        .expect("replica A setup has no binding");
    let project_id_a = first.project.local_project_id.clone();
    let bundle_id = first.project.bundle_id.clone();
    run_claude(
        &mut process_count,
        &provider,
        &claude_home_a,
        &process_home,
        &project_a,
        SOURCE_SESSION_ID,
        true,
        "MALLARD_CLAUDE_USER_R1",
        "MALLARD_CLAUDE_BASE",
    );
    let r1_history = marker_turn("MALLARD_CLAUDE_USER_R1", "MALLARD_CLAUDE_BASE");
    assert_transcript_structure_and_history(
        &claude_home_a,
        &project_a,
        SOURCE_SESSION_ID,
        &r1_history,
        4,
        &read_marker_path,
    );
    let source_r1_bytes = transcript_bytes(&claude_home_a, &project_a, SOURCE_SESSION_ID);
    let decoy_path = transcript_path(&claude_home_a, &project_a, FOREIGN_DECOY_SESSION_ID);
    write_foreign_cwd_decoy(&source_r1_bytes, &decoy_path, &foreign_project);
    assert_eq!(
        transcript_path(&claude_home_a, &foreign_project, FOREIGN_DECOY_SESSION_ID,),
        decoy_path,
        "foreign Claude project did not collide in the same physical bucket"
    );
    assert_transcript_structure_and_history(
        &claude_home_a,
        &foreign_project,
        FOREIGN_DECOY_SESSION_ID,
        &r1_history,
        4,
        &read_marker_path,
    );
    let pre_push_activity = wait_for_activity_workspace(
        &service_a,
        &project_id_a,
        &storage_id,
        activity_before_time,
        |workspace| {
            workspace.activity.sessions.len() == 1
                && workspace.activity.sessions[0]
                    .provider_session_id
                    .as_deref()
                    == Some(SOURCE_SESSION_ID)
        },
    );
    assert_eq!(pre_push_activity.activity.sessions.len(), 1);
    let pre_push_session = &pre_push_activity.activity.sessions[0];
    assert_eq!(
        pre_push_session.provider_session_id.as_deref(),
        Some(SOURCE_SESSION_ID)
    );
    assert_eq!(pre_push_session.source, SessionSourceState::LocalOnly);
    assert_activity_counts(pre_push_session, 1, 1, 1);
    assert_activity_matches_session_detail(&service_a, &project_id_a, pre_push_session);
    let pre_push_comparison = pre_push_activity
        .comparison
        .as_ref()
        .expect("pre-Push Activity workspace has no comparison");
    assert_eq!(pre_push_comparison.entries.len(), 1);
    assert_eq!(
        pre_push_comparison.entries[0].state,
        ThreadSyncState::LocalOnly
    );
    assert_eq!(
        pre_push_comparison.entries[0].provider_thread_id.as_deref(),
        Some(SOURCE_SESSION_ID),
        "foreign-cwd decoy entered the pre-Push comparison"
    );
    let first_push = push(&service_a, &project_id_a, &storage_id);
    assert_eq!(first_push.revision, Some(2));
    let logical_primary_a = logical_thread_id(&repository_a, &project_id_a, SOURCE_SESSION_ID);
    let cloud_primary = cloud_thread_id_of(&repository_a, &project_id_a, &logical_primary_a);
    assert_identity_namespaces_distinct(
        logical_primary_a.as_str(),
        SOURCE_SESSION_ID,
        cloud_primary.as_str(),
        "replica-A content-revision-2 conversation",
    );
    let r1_activity =
        activity_workspace(&service_a, &project_id_a, &storage_id, activity_before_time);
    assert_eq!(r1_activity.activity.sessions.len(), 1);
    assert!(r1_activity.activity.warnings.is_empty());
    let r1_activity_session = &r1_activity.activity.sessions[0];
    assert_eq!(
        r1_activity_session.source,
        SessionSourceState::LocalAndRemote
    );
    assert_eq!(
        r1_activity_session.provider_session_id.as_deref(),
        Some(SOURCE_SESSION_ID)
    );
    assert_eq!(
        r1_activity_session.logical_thread_id.as_deref(),
        Some(logical_primary_a.as_str())
    );
    assert_activity_counts(r1_activity_session, 1, 1, 1);
    let r1_comparison = r1_activity
        .comparison
        .as_ref()
        .expect("R1 Activity workspace has no comparison");
    assert_eq!(r1_comparison.entries.len(), 1);
    assert_eq!(r1_comparison.entries[0].state, ThreadSyncState::Synced);
    assert_eq!(
        r1_comparison.entries[0].provider_thread_id.as_deref(),
        Some(SOURCE_SESSION_ID),
        "foreign-cwd decoy entered the pushed comparison"
    );
    let r1_snapshot = redacted(
        service_a.inspect_bundle(&storage_id, &bundle_id),
        "cannot inspect content-revision-2 Claude bundle",
    );
    let pushed_source_ids = r1_snapshot
        .manifest
        .all_conversations()
        .filter_map(|(_, _, thread)| thread.source_thread_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        pushed_source_ids,
        BTreeSet::from([SOURCE_SESSION_ID.to_string()]),
        "foreign-cwd decoy entered the pushed bundle"
    );
    std::fs::remove_file(&decoy_path).expect("remove bounded foreign-cwd decoy fixture");
    assert_eq!(
        root_level_transcript_stems(&claude_home_a, &project_a),
        BTreeSet::from([SOURCE_SESSION_ID.to_string()]),
        "replica A did not have exactly one first-content Claude transcript"
    );
    let r1_resource = mallard_domain::conversation_resource_id(&claude_key(), &logical_primary_a)
        .expect("derive first-content Claude resource ID");
    session_detail_history(
        &service_a,
        &project_id_a,
        &r1_resource,
        SOURCE_SESSION_ID,
        &r1_history,
        &project_a,
        &read_marker_path,
    );
    let mut r1_machine_identity_needles = identity_needles([
        project_id_a.as_str(),
        logical_primary_a.as_str(),
        storage_id.as_str(),
        binding_a.host_id.as_str(),
        binding_a.replica_id.as_str(),
    ]);
    r1_machine_identity_needles.extend(
        binding_a
            .profile_ids
            .values()
            .map(|profile_id| profile_id.as_str().as_bytes().to_vec()),
    );
    r1_machine_identity_needles.sort();
    r1_machine_identity_needles.dedup();
    assert_portable_state_is_safe(
        &service_a,
        &storage_id,
        &bundle_id,
        &storage_root,
        &store_sensitive,
        &path_needles,
        &r1_machine_identity_needles,
    );
    // Positive control for the redaction scan: the stored transcript object
    // carries the typed placeholders where the planted credentials were.
    assert_storage_contains(&storage_root, b"<anthropic-api-key>");
    assert_storage_contains(&storage_root, b"DB_PASSWORD=<password>");

    // Content revision 3: B pulls revision 2, receives machine-local
    // identities, resumes the relocated transcript, and publishes it.
    let second = finish_setup(
        &repository_b,
        &service_b,
        &project_b,
        &claude_home_b,
        storage.clone(),
        DraftRepositoryChoice::Existing {
            storage_id: storage_id.clone(),
            bundle_id: bundle_id.clone(),
            display_name: first.project.display_name.clone(),
            repository_fingerprint: first.project.repository_fingerprint.clone(),
            mismatch_acknowledged: true,
        },
    );
    let binding_b = second
        .binding
        .clone()
        .expect("replica B setup has no binding");
    let project_id_b = second.project.local_project_id.clone();
    let before_initial_pull =
        activity_workspace(&service_b, &project_id_b, &storage_id, activity_before_time);
    assert_eq!(before_initial_pull.activity.sessions.len(), 1);
    let remote_only = &before_initial_pull.activity.sessions[0];
    assert_eq!(remote_only.source, SessionSourceState::RemoteOnly);
    assert_eq!(
        remote_only.provider_session_id.as_deref(),
        Some(SOURCE_SESSION_ID)
    );
    assert_activity_counts(remote_only, 1, 1, 1);
    let storage_only_comparison = before_initial_pull
        .comparison
        .as_ref()
        .expect("pre-Pull Activity workspace has no comparison");
    assert_eq!(storage_only_comparison.entries.len(), 1);
    assert_eq!(
        storage_only_comparison.entries[0].state,
        ThreadSyncState::StorageOnly
    );
    let (_, initial_pull) = pull_and_apply(&service_b, &storage_id, &bundle_id, &binding_b);
    assert!(initial_pull.preserved_threads.is_empty());
    let logical_primary_b = local_thread_of_cloud(&repository_b, &project_id_b, &cloud_primary);
    assert_ne!(logical_primary_b, logical_primary_a);
    let provider_primary_b = provider_thread_id(&repository_b, &project_id_b, &logical_primary_b);
    assert_ne!(provider_primary_b, SOURCE_SESSION_ID);
    assert_uuid_v4(
        &provider_primary_b,
        "replica-B initial materialized Claude provider ID",
    );
    assert_identity_namespaces_distinct(
        logical_primary_b.as_str(),
        &provider_primary_b,
        cloud_primary.as_str(),
        "replica-B content-revision-2 conversation",
    );
    let after_initial_pull = wait_for_activity_workspace(
        &service_b,
        &project_id_b,
        &storage_id,
        activity_before_time,
        |workspace| {
            workspace.activity.sessions.len() == 1
                && workspace.activity.sessions[0].source == SessionSourceState::LocalAndRemote
                && workspace.activity.sessions[0]
                    .provider_session_id
                    .as_deref()
                    == Some(provider_primary_b.as_str())
        },
    );
    let materialized_activity = &after_initial_pull.activity.sessions[0];
    assert_eq!(
        materialized_activity.logical_thread_id.as_deref(),
        Some(logical_primary_b.as_str())
    );
    assert_activity_counts(materialized_activity, 1, 1, 1);
    let materialized_comparison = after_initial_pull
        .comparison
        .as_ref()
        .expect("post-Pull Activity workspace has no comparison");
    assert_eq!(materialized_comparison.entries.len(), 1);
    assert_eq!(
        materialized_comparison.entries[0].state,
        ThreadSyncState::Synced
    );
    assert_eq!(
        root_level_transcript_stems(&claude_home_b, &project_b),
        BTreeSet::from([provider_primary_b.clone()]),
        "replica B did not have exactly one materialized Claude transcript"
    );
    assert_transcript_structure_and_history(
        &claude_home_b,
        &project_b,
        &provider_primary_b,
        &r1_history,
        4,
        &read_marker_path,
    );
    let destination_r1_bytes = transcript_bytes(&claude_home_b, &project_b, &provider_primary_b);
    // B materializes the relocated wire form: identical chain identities,
    // source-root paths decoded to B, and credential content redacted.
    assert_eq!(
        lineage_of(&destination_r1_bytes),
        relocated_lineage_of(
            &source_r1_bytes,
            &project_a,
            &project_b,
            &process_home,
            SOURCE_SESSION_ID,
            &provider_primary_b,
        ),
        "production Claude relocation changed lineage or semantics beyond path mapping and redaction"
    );
    assert_excludes(
        &destination_r1_bytes,
        &[
            PLANTED_LIVE_ANTHROPIC_KEY.as_bytes().to_vec(),
            PLANTED_LIVE_PASSWORD.as_bytes().to_vec(),
        ],
        "replica-B materialized transcript",
    );
    assert!(
        byte_slice_contains(&destination_r1_bytes, b"<anthropic-api-key>"),
        "replica-B materialized transcript lost its typed placeholder"
    );
    assert_eq!(
        transcript_bytes(&claude_home_a, &project_a, SOURCE_SESSION_ID),
        source_r1_bytes,
        "replica-B Pull mutated replica A's authoritative transcript"
    );
    run_claude(
        &mut process_count,
        &provider,
        &claude_home_b,
        &process_home,
        &project_b,
        &provider_primary_b,
        false,
        "MALLARD_CLAUDE_USER_R2",
        "MALLARD_CLAUDE_REMOTE_BRANCH",
    );
    let mut r2_history = r1_history.clone();
    r2_history.extend(marker_turn(
        "MALLARD_CLAUDE_USER_R2",
        "MALLARD_CLAUDE_REMOTE_BRANCH",
    ));
    assert_transcript_structure_and_history(
        &claude_home_b,
        &project_b,
        &provider_primary_b,
        &r2_history,
        6,
        &read_marker_path,
    );
    let r2_resource = mallard_domain::conversation_resource_id(&claude_key(), &logical_primary_b)
        .expect("derive replica-B R2 Session Detail resource ID");
    session_detail_history(
        &service_b,
        &project_id_b,
        &r2_resource,
        &provider_primary_b,
        &r2_history,
        &project_b,
        &read_marker_path,
    );
    let r2_activity =
        activity_workspace(&service_b, &project_id_b, &storage_id, activity_before_time);
    let r2_activity_session = r2_activity
        .activity
        .sessions
        .iter()
        .find(|session| session.resource_id == r2_resource.as_str())
        .expect("forced Activity refresh omitted replica-B R2");
    let (r2_users, r2_agents, r2_tools) = marker_history_activity_counts(&r2_history);
    assert_activity_counts(r2_activity_session, r2_users, r2_agents, r2_tools);
    assert!(r2_activity
        .comparison
        .as_ref()
        .expect("forced R2 Activity workspace has no comparison")
        .entries
        .iter()
        .any(|entry| {
            entry.cloud_thread_id.as_ref() == Some(&cloud_primary)
                && entry.state == ThreadSyncState::LocalAhead
        }));
    let second_push = push(&service_b, &project_id_b, &storage_id);
    assert_eq!(second_push.revision, Some(3));

    // Content revision 4: A advances its old base, is push-gated, and keeps both.
    run_claude(
        &mut process_count,
        &provider,
        &claude_home_a,
        &process_home,
        &project_a,
        SOURCE_SESSION_ID,
        false,
        "MALLARD_CLAUDE_USER_R3",
        "MALLARD_CLAUDE_LOCAL_BRANCH",
    );
    let mut r3_history = r1_history.clone();
    r3_history.extend(marker_turn(
        "MALLARD_CLAUDE_USER_R3",
        "MALLARD_CLAUDE_LOCAL_BRANCH",
    ));
    let local_r3_before_keep = transcript_bytes(&claude_home_a, &project_a, SOURCE_SESSION_ID);
    assert_divergent_push_gate(
        &service_a,
        &project_id_a,
        &storage_id,
        &cloud_primary,
        "replica A",
    );
    let divergent_plan_a = redacted(
        service_a.plan_bundle_restore(&storage_id, &bundle_id, &binding_a),
        "cannot plan replica-A divergent Claude Pull",
    );
    assert_keep_both_offered(&divergent_plan_a, &logical_primary_a);
    let divergent_apply_a = redacted(
        service_a.apply_bundle_restore(&divergent_plan_a, approved_pull_actions(&divergent_plan_a)),
        "cannot apply replica-A Claude Keep Both",
    );
    assert!(divergent_apply_a.success);
    let first_keep = assert_keep_both_result(
        &repository_a,
        &project_id_a,
        &logical_primary_a,
        &cloud_primary,
        SOURCE_SESSION_ID,
        &divergent_apply_a,
    );
    assert_uuid_v4(
        &first_keep.remote_provider_thread_id,
        "first Keep Both remote Claude provider ID",
    );
    assert_eq!(
        transcript_bytes(&claude_home_a, &project_a, SOURCE_SESSION_ID),
        local_r3_before_keep,
        "Keep Both rewrote the preserved local Claude transcript"
    );
    assert_transcript_structure_and_history(
        &claude_home_a,
        &project_a,
        SOURCE_SESSION_ID,
        &r3_history,
        6,
        &read_marker_path,
    );
    assert_transcript_structure_and_history(
        &claude_home_a,
        &project_a,
        &first_keep.remote_provider_thread_id,
        &r2_history,
        6,
        &read_marker_path,
    );
    let third_push = push(&service_a, &project_id_a, &storage_id);
    assert_eq!(third_push.revision, Some(4));
    let preserved_a_cloud = cloud_thread_id_of(
        &repository_a,
        &project_id_a,
        &first_keep.preserved_thread_id,
    );
    assert_ne!(preserved_a_cloud, cloud_primary);

    // B pulls content revision 4, then both replicas advance the shared
    // primary in opposite directions. A wins revision 5; B must keep both.
    let (_, third_pull) = pull_and_apply(&service_b, &storage_id, &bundle_id, &binding_b);
    assert!(third_pull.preserved_threads.is_empty());
    assert_eq!(
        provider_thread_id(&repository_b, &project_id_b, &logical_primary_b),
        provider_primary_b,
        "non-Codex MaterializeCloud replaced B's stable provider ID"
    );
    let preserved_a_local_b =
        local_thread_of_cloud(&repository_b, &project_id_b, &preserved_a_cloud);
    let preserved_a_provider_b =
        provider_thread_id(&repository_b, &project_id_b, &preserved_a_local_b);
    assert_uuid_v4(
        &preserved_a_provider_b,
        "replica-B preserved-lineage Claude provider ID",
    );
    assert_transcript_structure_and_history(
        &claude_home_b,
        &project_b,
        &preserved_a_provider_b,
        &r3_history,
        6,
        &read_marker_path,
    );
    run_claude(
        &mut process_count,
        &provider,
        &claude_home_a,
        &process_home,
        &project_a,
        &first_keep.remote_provider_thread_id,
        false,
        "MALLARD_CLAUDE_USER_R4",
        "MALLARD_CLAUDE_REVISION_FOUR",
    );
    run_claude(
        &mut process_count,
        &provider,
        &claude_home_b,
        &process_home,
        &project_b,
        &provider_primary_b,
        false,
        "MALLARD_CLAUDE_USER_R5",
        "MALLARD_CLAUDE_SECOND_LOCAL_BRANCH",
    );
    let mut r4_history = r2_history.clone();
    r4_history.extend(marker_turn(
        "MALLARD_CLAUDE_USER_R4",
        "MALLARD_CLAUDE_REVISION_FOUR",
    ));
    let mut r5_history = r2_history.clone();
    r5_history.extend(marker_turn(
        "MALLARD_CLAUDE_USER_R5",
        "MALLARD_CLAUDE_SECOND_LOCAL_BRANCH",
    ));
    session_detail_history(
        &service_a,
        &project_id_a,
        &r1_resource,
        &first_keep.remote_provider_thread_id,
        &r4_history,
        &project_a,
        &read_marker_path,
    );
    session_detail_history(
        &service_b,
        &project_id_b,
        &r2_resource,
        &provider_primary_b,
        &r5_history,
        &project_b,
        &read_marker_path,
    );
    let r4_activity =
        activity_workspace(&service_a, &project_id_a, &storage_id, activity_before_time);
    assert!(r4_activity
        .comparison
        .as_ref()
        .expect("forced R4 Activity workspace has no comparison")
        .entries
        .iter()
        .any(|entry| {
            entry.cloud_thread_id.as_ref() == Some(&cloud_primary)
                && entry.provider_thread_id.as_deref()
                    == Some(first_keep.remote_provider_thread_id.as_str())
                && entry.state == ThreadSyncState::LocalAhead
        }));
    let r5_activity =
        activity_workspace(&service_b, &project_id_b, &storage_id, activity_before_time);
    assert!(r5_activity
        .comparison
        .as_ref()
        .expect("forced R5 Activity workspace has no comparison")
        .entries
        .iter()
        .any(|entry| {
            entry.cloud_thread_id.as_ref() == Some(&cloud_primary)
                && entry.provider_thread_id.as_deref() == Some(provider_primary_b.as_str())
                && entry.state == ThreadSyncState::LocalAhead
        }));
    let fourth_push = push(&service_a, &project_id_a, &storage_id);
    assert_eq!(fourth_push.revision, Some(5));
    assert_divergent_push_gate(
        &service_b,
        &project_id_b,
        &storage_id,
        &cloud_primary,
        "replica B",
    );
    let local_r5_before_keep = transcript_bytes(&claude_home_b, &project_b, &provider_primary_b);
    let divergent_plan_b = redacted(
        service_b.plan_bundle_restore(&storage_id, &bundle_id, &binding_b),
        "cannot plan replica-B divergent Claude Pull",
    );
    assert_keep_both_offered(&divergent_plan_b, &logical_primary_b);
    let divergent_apply_b = redacted(
        service_b.apply_bundle_restore(&divergent_plan_b, approved_pull_actions(&divergent_plan_b)),
        "cannot apply replica-B Claude Keep Both",
    );
    assert!(divergent_apply_b.success);
    let second_keep = assert_keep_both_result(
        &repository_b,
        &project_id_b,
        &logical_primary_b,
        &cloud_primary,
        &provider_primary_b,
        &divergent_apply_b,
    );
    assert_uuid_v4(
        &second_keep.remote_provider_thread_id,
        "second Keep Both remote Claude provider ID",
    );
    assert_eq!(
        transcript_bytes(&claude_home_b, &project_b, &provider_primary_b),
        local_r5_before_keep,
        "reverse Keep Both rewrote B's preserved Claude transcript"
    );
    assert_transcript_structure_and_history(
        &claude_home_b,
        &project_b,
        &second_keep.remote_provider_thread_id,
        &r4_history,
        8,
        &read_marker_path,
    );
    assert_transcript_structure_and_history(
        &claude_home_b,
        &project_b,
        &provider_primary_b,
        &r5_history,
        8,
        &read_marker_path,
    );

    // Content revision 6 and fixed point: publish B's preserved branch, pull
    // it to A, then require reviewed no-op Pull/Push and byte-identical storage.
    let fifth_push = push(&service_b, &project_id_b, &storage_id);
    assert_eq!(fifth_push.revision, Some(6));
    let preserved_b_cloud = cloud_thread_id_of(
        &repository_b,
        &project_id_b,
        &second_keep.preserved_thread_id,
    );
    assert_ne!(preserved_b_cloud, cloud_primary);
    assert_ne!(preserved_b_cloud, preserved_a_cloud);
    let (_, fifth_pull) = pull_and_apply(&service_a, &storage_id, &bundle_id, &binding_a);
    assert!(fifth_pull.preserved_threads.is_empty());
    let preserved_b_local_a =
        local_thread_of_cloud(&repository_a, &project_id_a, &preserved_b_cloud);
    let preserved_b_provider_a =
        provider_thread_id(&repository_a, &project_id_a, &preserved_b_local_a);
    assert_uuid_v4(
        &preserved_b_provider_a,
        "replica-A preserved-lineage Claude provider ID",
    );
    assert_transcript_structure_and_history(
        &claude_home_a,
        &project_a,
        &preserved_b_provider_a,
        &r5_history,
        8,
        &read_marker_path,
    );
    let expected_cloud_ids = [&cloud_primary, &preserved_a_cloud, &preserved_b_cloud]
        .into_iter()
        .map(|cloud_id| cloud_id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(expected_cloud_ids.len(), 3);
    let storage_before_noops = tree_digest_inventory(&storage_root);
    assert_pull_is_no_op(&service_a, &storage_id, &bundle_id, &binding_a, 6);
    assert_pull_is_no_op(&service_b, &storage_id, &bundle_id, &binding_b, 6);
    for (service, project_id) in [(&service_a, &project_id_a), (&service_b, &project_id_b)] {
        let no_op = push(service, project_id, &storage_id);
        assert_eq!(no_op.revision, Some(6));
        assert_eq!(no_op.resources_changed, Some(0));
    }
    assert_eq!(tree_digest_inventory(&storage_root), storage_before_noops);
    assert_eq!(
        redacted(
            service_a.inspect_bundle(&storage_id, &bundle_id),
            "cannot inspect final Claude bundle",
        )
        .head
        .revision,
        6
    );
    let topology_a = assert_exact_replica_topology(
        &repository_a,
        &service_a,
        &binding_a,
        &project_id_a,
        &storage_id,
        &claude_home_a,
        &project_a,
        &expected_cloud_ids,
        6,
        "replica A",
    );
    let topology_b = assert_exact_replica_topology(
        &repository_b,
        &service_b,
        &binding_b,
        &project_id_b,
        &storage_id,
        &claude_home_b,
        &project_b,
        &expected_cloud_ids,
        6,
        "replica B",
    );
    assert_cross_replica_topology(&topology_a, &topology_b, &expected_cloud_ids);
    assert_eq!(
        process_count, EXPECTED_SOURCE_PROCESSES,
        "Claude source process budget changed"
    );

    let final_histories = vec![
        (cloud_primary.clone(), r4_history),
        (preserved_a_cloud.clone(), r3_history),
        (preserved_b_cloud.clone(), r5_history),
    ];
    assert_eq!(
        final_histories
            .iter()
            .map(|(_, history)| history.clone())
            .collect::<Vec<_>>(),
        final_history_oracles(),
        "final Claude lineage oracle changed"
    );
    let final_activity_a_plain =
        activity_workspace(&service_a, &project_id_a, &storage_id, activity_before_time);
    let final_activity_a_cached =
        activity_workspace(&service_a, &project_id_a, &storage_id, activity_before_time);
    assert_eq!(
        stable_activity_workspace(&final_activity_a_plain),
        stable_activity_workspace(&final_activity_a_cached),
        "replica-A plain and cached Activity projections differ"
    );
    assert_final_activity_workspace(
        &final_activity_a_cached,
        &topology_a,
        &final_histories,
        &cloud_primary,
        6,
        "replica A",
    );

    let final_activity_b_plain =
        activity_workspace(&service_b, &project_id_b, &storage_id, activity_before_time);
    let final_activity_b_cached =
        activity_workspace(&service_b, &project_id_b, &storage_id, activity_before_time);
    assert_eq!(
        stable_activity_workspace(&final_activity_b_plain),
        stable_activity_workspace(&final_activity_b_cached),
        "replica-B plain and cached Activity projections differ"
    );
    assert_final_activity_workspace(
        &final_activity_b_cached,
        &topology_b,
        &final_histories,
        &cloud_primary,
        6,
        "replica B",
    );
    verify_final_histories(
        &mut process_count,
        &provider,
        &sensitive,
        &read_marker_path,
        [
            (
                &repository_a,
                &service_a,
                &project_id_a,
                &claude_home_a,
                &project_a,
            ),
            (
                &repository_b,
                &service_b,
                &project_id_b,
                &claude_home_b,
                &project_b,
            ),
        ],
        &final_histories,
    );
    assert_eq!(
        process_count, EXPECTED_MODEL_TURN_PROCESSES,
        "Claude model-turn process budget changed"
    );

    // Start a fresh storage epoch through the production clear operation.
    // The remote delete occurs once; both replicas separately discard their
    // accepted bases before the synchronized local sessions are republished.
    eprintln!("live Claude phase: storage reset and session republish");
    let claude_home_a_before_reset = tree_digest_inventory(&claude_home_a);
    let claude_home_b_before_reset = tree_digest_inventory(&claude_home_b);
    let project_before_reset = redacted(
        service_a.inspect_bundle(&storage_id, &bundle_id),
        "cannot inspect revision-6 Claude bundle before storage reset",
    );
    let conversations_before_reset = project_before_reset
        .manifest
        .all_conversations()
        .map(|(_, cloud_id, thread)| {
            (
                cloud_id.clone(),
                (
                    thread.provider.clone(),
                    thread.lineage_digest.clone(),
                    thread.semantic_digest.clone(),
                    thread.message_count,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(conversations_before_reset.len(), 3);

    let reset_a = clear_storage_for_replica(&repository_a, &service_a, &storage_id);
    assert!(reset_a.repository_objects_deleted > 0);
    assert_eq!(reset_a.library_objects_deleted, 0);
    assert_eq!(reset_a.project_links_reset, 1);
    assert_eq!(reset_a.catalog_links_reset, 0);
    let reset_b = clear_storage_for_replica(&repository_b, &service_b, &storage_id);
    assert_eq!(reset_b.repository_objects_deleted, 0);
    assert_eq!(reset_b.library_objects_deleted, 0);
    assert_eq!(reset_b.project_links_reset, 1);
    assert_eq!(reset_b.catalog_links_reset, 0);
    assert!(service_a.inspect_bundle(&storage_id, &bundle_id).is_err());
    assert_eq!(
        tree_digest_inventory(&claude_home_a),
        claude_home_a_before_reset
    );
    assert_eq!(
        tree_digest_inventory(&claude_home_b),
        claude_home_b_before_reset
    );

    let reset_push = push(&service_a, &project_id_a, &storage_id);
    assert!(reset_push.success, "{}", reset_push.message);
    assert_eq!(reset_push.revision, Some(1));
    let (_, reset_pull) = pull_and_apply(&service_b, &storage_id, &bundle_id, &binding_b);
    assert!(reset_pull.preserved_threads.is_empty());
    let project_after_reset = redacted(
        service_a.inspect_bundle(&storage_id, &bundle_id),
        "cannot inspect republished Claude bundle",
    );
    assert_eq!(project_after_reset.head.revision, 1);
    let conversations_after_reset = project_after_reset
        .manifest
        .all_conversations()
        .map(|(_, cloud_id, thread)| {
            (
                cloud_id.clone(),
                (
                    thread.provider.clone(),
                    thread.lineage_digest.clone(),
                    thread.semantic_digest.clone(),
                    thread.message_count,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(conversations_after_reset, conversations_before_reset);
    let reset_topology_a = assert_exact_replica_topology(
        &repository_a,
        &service_a,
        &binding_a,
        &project_id_a,
        &storage_id,
        &claude_home_a,
        &project_a,
        &expected_cloud_ids,
        1,
        "replica A after storage reset",
    );
    let reset_topology_b = assert_exact_replica_topology(
        &repository_b,
        &service_b,
        &binding_b,
        &project_id_b,
        &storage_id,
        &claude_home_b,
        &project_b,
        &expected_cloud_ids,
        1,
        "replica B after storage reset",
    );
    assert_eq!(reset_topology_a.by_cloud, topology_a.by_cloud);
    assert_eq!(reset_topology_b.by_cloud, topology_b.by_cloud);
    assert_cross_replica_topology(&reset_topology_a, &reset_topology_b, &expected_cloud_ids);
    assert_eq!(
        tree_digest_inventory(&claude_home_a),
        claude_home_a_before_reset
    );
    assert_eq!(
        tree_digest_inventory(&claude_home_b),
        claude_home_b_before_reset
    );

    // Cursor parity: a warm comparison reads zero Claude transcript bytes,
    // a real resumed CLI turn extends the checkpoint cursor through the
    // probe window plus the appended bytes only, and the extended
    // observation agrees with a from-scratch analysis of the final bytes.
    let canonical_home_a =
        std::fs::canonicalize(&claude_home_a).expect("canonicalize replica-A Claude home");
    let _ = redacted(
        service_a.compare_threads(&project_id_a, &storage_id),
        "cannot warm the replica-A comparison caches",
    );
    let metrics = mallard_infrastructure::sync_metrics::register_root(&canonical_home_a);
    let warm = redacted(
        service_a.compare_threads(&project_id_a, &storage_id),
        "cannot run the warm replica-A comparison",
    );
    assert_eq!(warm.entries.len(), 3, "replica-A final row count changed");
    let counted = metrics.snapshot();
    assert_eq!(
        counted.transcript_bytes_read, 0,
        "a warm Claude comparison reads no transcript bytes"
    );
    assert_eq!(counted.full_graph_resolves, 0, "the cursor rides the probe");
    assert_eq!(counted.selected_captures, 0, "no capture on a warm refresh");

    let primary_local_a = local_thread_of_cloud(&repository_a, &project_id_a, &cloud_primary);
    let primary_provider_id_a = provider_thread_id(&repository_a, &project_id_a, &primary_local_a);
    let base_transcript = transcript_bytes(&claude_home_a, &project_a, &primary_provider_id_a);
    let base_chain_len = match lineage_of(&base_transcript) {
        mallard_infrastructure::conversation_codec::CodecLineage::Complete { chain, .. } => {
            chain.len()
        }
        other => panic!("primary transcript lineage is not complete: {other:?}"),
    };
    let primary_resource_a =
        mallard_domain::conversation_resource_id(&claude_key(), &primary_local_a)
            .expect("derive primary cursor-append Activity resource ID");
    let activity_before_append =
        activity_workspace(&service_a, &project_id_a, &storage_id, activity_before_time);
    let primary_activity_before = activity_before_append
        .activity
        .sessions
        .iter()
        .find(|session| session.resource_id == primary_resource_a.as_str())
        .expect("primary Activity row before the appended turn");
    let activity_metrics_before = activity_before_append
        .activity
        .sessions
        .iter()
        .map(|session| {
            (
                session.resource_id.clone(),
                (
                    session.user_round_count,
                    session.agent_message_count,
                    session.tool_call_count,
                    session.total_input_tokens,
                    session.total_output_tokens,
                    session.total_tokens,
                    session.metrics_complete,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let detail_before_append = service_a
        .get_project_session_detail(&project_id_a, &primary_resource_a)
        .expect("cannot load Session Detail before the appended Claude turn");
    run_claude(
        &mut process_count,
        &provider,
        &claude_home_a,
        &process_home,
        &project_a,
        &primary_provider_id_a,
        false,
        "MALLARD_CLAUDE_USER_CURSOR_APPEND",
        "MALLARD_CLAUDE_CURSOR_EXTENDED",
    );
    wait_for_transcript_marker(
        &claude_home_a,
        &project_a,
        &primary_provider_id_a,
        "MALLARD_CLAUDE_CURSOR_EXTENDED",
    );
    let extended_transcript = transcript_bytes(&claude_home_a, &project_a, &primary_provider_id_a);
    assert_eq!(
        &extended_transcript[..base_transcript.len()],
        &base_transcript[..],
        "the resumed turn must append, never rewrite"
    );
    // A comparison serves its watcher-clean observation until the watcher
    // reports the append, and that delivery is asynchronous: poll for the
    // comparison that consumes it. Every earlier comparison serves the cache
    // with no reads; the consuming one reads only the appended bytes.
    let appended_bytes = (extended_transcript.len() - base_transcript.len()) as u64;
    let deadline = Instant::now() + TRANSCRIPT_SETTLE_TIMEOUT;
    let extended = loop {
        let before_extension = metrics.snapshot();
        let extended = redacted(
            service_a.compare_threads(&project_id_a, &storage_id),
            "cannot compare after the appended Claude turn",
        );
        let delta = metrics.snapshot().delta_since(&before_extension);
        assert_eq!(
            delta.transcript_bytes_read, 0,
            "the appended turn rides the cursor, never a whole-file read"
        );
        assert_eq!(delta.full_graph_resolves, 0);
        if delta.append_bytes_read >= appended_bytes {
            break extended;
        }
        assert_eq!(
            delta.append_bytes_read, 0,
            "a comparison ahead of the watcher serves its cache untouched"
        );
        assert!(
            Instant::now() < deadline,
            "the watcher did not report the appended Claude turn before its bounded deadline"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    let primary_row = extended
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&cloud_primary))
        .expect("primary row after the appended turn");
    assert_eq!(
        primary_row.state,
        ThreadSyncState::LocalAhead,
        "a resumed turn is locally outstanding"
    );
    match lineage_of(&extended_transcript) {
        mallard_infrastructure::conversation_codec::CodecLineage::Complete { chain, .. } => {
            assert_eq!(
                chain.len(),
                base_chain_len + 2,
                "one resumed turn appends exactly one user and one assistant record"
            );
        }
        other => panic!("extended transcript lineage is not complete: {other:?}"),
    }
    // The persisted cursor serves the next comparison with zero reads and
    // identical rows — incremental and from-scratch agree.
    let before_settled = metrics.snapshot();
    let settled = redacted(
        service_a.compare_threads(&project_id_a, &storage_id),
        "cannot re-compare the settled replica-A state",
    );
    let delta = metrics.snapshot().delta_since(&before_settled);
    assert_eq!(delta.transcript_bytes_read, 0);
    assert_eq!(
        delta.append_bytes_read, 0,
        "the advanced cursor is persisted"
    );
    assert_eq!(delta.full_graph_resolves, 0);
    assert_eq!(
        serde_json::to_value(&settled.entries).expect("serialize settled rows"),
        serde_json::to_value(&extended.entries).expect("serialize extended rows"),
        "cursor-extended and settled comparisons produce identical rows"
    );

    let activity_after_append = wait_for_activity_workspace(
        &service_a,
        &project_id_a,
        &storage_id,
        activity_before_time,
        |workspace| {
            workspace.activity.sessions.iter().any(|session| {
                session.resource_id == primary_resource_a.as_str()
                    && session.user_round_count
                        == primary_activity_before.user_round_count.saturating_add(1)
                    && session.agent_message_count
                        == primary_activity_before
                            .agent_message_count
                            .saturating_add(1)
                    && session.tool_call_count == primary_activity_before.tool_call_count
            })
        },
    );
    assert_eq!(
        activity_after_append
            .activity
            .sessions
            .iter()
            .map(|session| session.resource_id.clone())
            .collect::<BTreeSet<_>>(),
        activity_metrics_before
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        "cursor append changed the Activity roster"
    );
    for session in &activity_after_append.activity.sessions {
        let before = activity_metrics_before
            .get(&session.resource_id)
            .expect("post-append Activity contains an unknown row");
        if session.resource_id == primary_resource_a.as_str() {
            assert_eq!(session.user_round_count, before.0 + 1);
            assert_eq!(session.agent_message_count, before.1 + 1);
            assert_eq!(session.tool_call_count, before.2);
            assert!(session.metrics_complete);
            assert_activity_token_totals(session);
            assert!(
                session.total_input_tokens > before.3,
                "the appended Claude turn did not increase input tokens"
            );
            assert!(
                session.total_output_tokens > before.4,
                "the appended Claude turn did not increase output tokens"
            );
            assert!(
                session.total_tokens > before.5,
                "the appended Claude turn did not increase total tokens"
            );
            assert_eq!(session.source, SessionSourceState::LocalAndRemote);
        } else {
            assert_eq!(
                (
                    session.user_round_count,
                    session.agent_message_count,
                    session.tool_call_count,
                    session.total_input_tokens,
                    session.total_output_tokens,
                    session.total_tokens,
                    session.metrics_complete,
                ),
                *before,
                "cursor append changed an unrelated Activity row"
            );
        }
    }

    let detail_after_append = service_a
        .get_project_session_detail(&project_id_a, &primary_resource_a)
        .expect("cannot refresh Session Detail after the appended Claude turn");
    assert_ne!(
        detail_after_append.snapshot_id, detail_before_append.snapshot_id,
        "Session Detail snapshot did not advance after an append"
    );
    let prompts_before_append = detail_before_append
        .event_counts
        .prompts
        .value
        .expect("pre-append Session Detail prompt count is unavailable");
    let responses_before_append = detail_before_append
        .event_counts
        .responses
        .value
        .expect("pre-append Session Detail response count is unavailable");
    assert_eq!(
        detail_after_append.event_counts.prompts.value,
        Some(prompts_before_append + 1)
    );
    assert_eq!(
        detail_after_append.event_counts.responses.value,
        Some(responses_before_append + 1)
    );
    assert_eq!(
        detail_after_append.event_counts.tool_calls,
        detail_before_append.event_counts.tool_calls
    );
    assert_eq!(
        detail_after_append.event_counts.reads,
        detail_before_append.event_counts.reads
    );
    let stale_snapshot_error = service_a
        .get_project_session_timeline_page(
            &project_id_a,
            &primary_resource_a,
            ProjectSessionTimelineRequest {
                snapshot_id: detail_before_append.snapshot_id.clone(),
                cursor: None,
                limit: Some(1),
                included_event_kinds: BTreeSet::from([SessionTimelineEventKind::Prompt]),
                included_tool_categories: BTreeSet::new(),
                turn_id: None,
            },
        )
        .expect_err("stale Session Detail snapshot unexpectedly loaded a page");
    assert_eq!(
        stale_snapshot_error.code,
        SessionDetailErrorCode::SnapshotChanged
    );
    assert_eq!(
        stale_snapshot_error.current_snapshot_id.as_deref(),
        Some(detail_after_append.snapshot_id.as_str())
    );
    assert_eq!(
        process_count, EXPECTED_CURSOR_APPEND_PROCESSES,
        "Claude cursor-append process budget changed"
    );

    let mut final_machine_identity_needles = identity_needles([
        project_id_a.as_str(),
        project_id_b.as_str(),
        logical_primary_a.as_str(),
        logical_primary_b.as_str(),
        first_keep.preserved_thread_id.as_str(),
        preserved_a_local_b.as_str(),
        second_keep.preserved_thread_id.as_str(),
        preserved_b_local_a.as_str(),
        storage_id.as_str(),
        binding_a.host_id.as_str(),
        binding_a.replica_id.as_str(),
        binding_b.host_id.as_str(),
        binding_b.replica_id.as_str(),
    ]);
    final_machine_identity_needles.extend(
        binding_a
            .profile_ids
            .values()
            .chain(binding_b.profile_ids.values())
            .map(|profile_id| profile_id.as_str().as_bytes().to_vec()),
    );
    final_machine_identity_needles.sort();
    final_machine_identity_needles.dedup();
    assert_portable_state_is_safe(
        &service_a,
        &storage_id,
        &bundle_id,
        &storage_root,
        &store_sensitive,
        &path_needles,
        &final_machine_identity_needles,
    );
    for (root, label) in [
        (&claude_home_a, "replica-A Claude home"),
        (&claude_home_b, "replica-B Claude home"),
        (&machine_a_home, "replica-A Mallard home"),
        (&machine_b_home, "replica-B Mallard home"),
        (&process_home, "Claude process home"),
    ] {
        assert_private_tree(root, &sensitive, label);
    }
    aimock
        .finish(&sensitive)
        .expect("Claude aimock audit did not prove the exact model/probe request contract");
}

#[test]
#[ignore = "runs three paid Claude Code turns through Tuzi; set MALLARD_RUN_LIVE_CLAUDE_TUZI_E2E=1"]
fn live_claude_tuzi_setup_push_pull_divergence() {
    if std::env::var(LIVE_TUZI_TEST_GATE).ok().as_deref() != Some("1") {
        return;
    }

    let root = tempfile::tempdir().expect("create isolated paid Claude live-test root");
    let process_home = root.path().join("process-home");
    let claude_home_a = root.path().join("claude-a");
    let claude_home_b = root.path().join("claude-b");
    let project_a_input = root.path().join("project-a");
    let project_b_input = root.path().join("project-b");
    let storage_root = root.path().join("storage");
    let machine_a_home = root.path().join("machine-a");
    let machine_b_home = root.path().join("machine-b");
    for directory in [
        &process_home,
        &claude_home_a,
        &claude_home_b,
        &project_a_input,
        &project_b_input,
        &storage_root,
        &machine_a_home,
        &machine_b_home,
    ] {
        secure_directory(directory);
    }
    let project_a = std::fs::canonicalize(&project_a_input)
        .expect("canonicalize replica-A paid Claude project");
    let project_b = std::fs::canonicalize(&project_b_input)
        .expect("canonicalize replica-B paid Claude project");
    let canonical_root =
        std::fs::canonicalize(root.path()).expect("canonicalize paid Claude live-test root");

    // Fail before reading the real credential if this native-format contract
    // has not been proven for the installed Claude Code version.
    assert_supported_claude_version(&process_home, EXPECTED_TUZI_CLAUDE_VERSION);
    let provider = load_live_tuzi_connection();
    let sensitive = provider.protected_needles();
    let _home_guard = EnvironmentGuard::set("HOME", &process_home);
    let _userprofile_guard = EnvironmentGuard::set("USERPROFILE", &process_home);

    let repository_a = redacted(
        LocalStateRepository::from_home_dir(&machine_a_home),
        "cannot create replica-A paid Claude repository",
    );
    let repository_b = redacted(
        LocalStateRepository::from_home_dir(&machine_b_home),
        "cannot create replica-B paid Claude repository",
    );
    let service_a = ProjectSyncService::new(&repository_a);
    let service_b = ProjectSyncService::new(&repository_b);
    let storage = live_tuzi_storage(&storage_root);
    let storage_id = storage.id.clone();
    let mut path_needles = vec![root.path().to_string_lossy().as_bytes().to_vec()];
    let canonical_needle = canonical_root.to_string_lossy().as_bytes().to_vec();
    if !path_needles.contains(&canonical_needle) {
        path_needles.push(canonical_needle);
    }
    let mut process_count = 0_usize;
    let mut reported_cost_usd = 0.0_f64;

    // Setup publishes registration-only revision 1. Content revision 2:
    // create one real, zero-tool Sonnet 5 session on A and publish it through
    // the reviewed production Push path.
    let first = finish_setup(
        &repository_a,
        &service_a,
        &project_a,
        &claude_home_a,
        storage.clone(),
        DraftRepositoryChoice::New,
    );
    let binding_a = first
        .binding
        .clone()
        .expect("replica A paid setup has no binding");
    let project_id_a = first.project.local_project_id.clone();
    let bundle_id = first.project.bundle_id.clone();
    reported_cost_usd += run_live_tuzi_claude(
        &mut process_count,
        &provider,
        &claude_home_a,
        &process_home,
        &project_a,
        LIVE_TUZI_SOURCE_SESSION_ID,
        true,
        "MALLARD_CLAUDE_LIVE_USER_R1",
        "MALLARD_CLAUDE_LIVE_BASE",
    );
    let r1_history = marker_turn("MALLARD_CLAUDE_LIVE_USER_R1", "MALLARD_CLAUDE_LIVE_BASE");
    assert_live_text_transcript(
        &claude_home_a,
        &project_a,
        LIVE_TUZI_SOURCE_SESSION_ID,
        &r1_history,
        2,
    );
    let source_r1_bytes = transcript_bytes(&claude_home_a, &project_a, LIVE_TUZI_SOURCE_SESSION_ID);
    let source_r1_lineage = lineage_of(&source_r1_bytes);
    let first_push = push(&service_a, &project_id_a, &storage_id);
    assert_eq!(first_push.revision, Some(2));
    let logical_primary_a =
        logical_thread_id(&repository_a, &project_id_a, LIVE_TUZI_SOURCE_SESSION_ID);
    let cloud_primary = cloud_thread_id_of(&repository_a, &project_id_a, &logical_primary_a);
    assert_identity_namespaces_distinct(
        logical_primary_a.as_str(),
        LIVE_TUZI_SOURCE_SESSION_ID,
        cloud_primary.as_str(),
        "replica-A paid content-revision-2 conversation",
    );
    assert_eq!(
        root_level_transcript_stems(&claude_home_a, &project_a),
        BTreeSet::from([LIVE_TUZI_SOURCE_SESSION_ID.to_string()])
    );
    let r1_resource = mallard_domain::conversation_resource_id(&claude_key(), &logical_primary_a)
        .expect("derive paid content-revision-2 Claude resource ID");
    session_detail_text_history(
        &service_a,
        &project_id_a,
        &r1_resource,
        LIVE_TUZI_SOURCE_SESSION_ID,
        &r1_history,
    );
    let mut r1_machine_identity_needles = identity_needles([
        project_id_a.as_str(),
        logical_primary_a.as_str(),
        storage_id.as_str(),
        binding_a.host_id.as_str(),
        binding_a.replica_id.as_str(),
    ]);
    r1_machine_identity_needles.extend(
        binding_a
            .profile_ids
            .values()
            .map(|profile_id| profile_id.as_str().as_bytes().to_vec()),
    );
    r1_machine_identity_needles.sort();
    r1_machine_identity_needles.dedup();
    assert_portable_state_is_safe(
        &service_a,
        &storage_id,
        &bundle_id,
        &storage_root,
        &sensitive,
        &path_needles,
        &r1_machine_identity_needles,
    );

    // Content revision 3: B performs a reviewed Pull, gets new local/provider
    // identities, resumes the production-relocated transcript through Tuzi,
    // and publishes the remote continuation.
    let second = finish_setup(
        &repository_b,
        &service_b,
        &project_b,
        &claude_home_b,
        storage.clone(),
        DraftRepositoryChoice::Existing {
            storage_id: storage_id.clone(),
            bundle_id: bundle_id.clone(),
            display_name: first.project.display_name.clone(),
            repository_fingerprint: first.project.repository_fingerprint.clone(),
            mismatch_acknowledged: true,
        },
    );
    let binding_b = second
        .binding
        .clone()
        .expect("replica B paid setup has no binding");
    let project_id_b = second.project.local_project_id.clone();
    let (initial_plan, initial_pull) =
        pull_and_apply(&service_b, &storage_id, &bundle_id, &binding_b);
    assert!(initial_pull.preserved_threads.is_empty());
    assert_eq!(
        initial_plan
            .conversation_review
            .iter()
            .filter(|row| row.action == ConversationReviewAction::CreateLocal)
            .count(),
        1,
        "initial paid Pull did not review exactly one local creation"
    );
    let logical_primary_b = local_thread_of_cloud(&repository_b, &project_id_b, &cloud_primary);
    assert_ne!(logical_primary_b, logical_primary_a);
    let provider_primary_b = provider_thread_id(&repository_b, &project_id_b, &logical_primary_b);
    assert_ne!(provider_primary_b, LIVE_TUZI_SOURCE_SESSION_ID);
    assert_uuid_v4(
        &provider_primary_b,
        "replica-B paid materialized Claude provider ID",
    );
    assert_identity_namespaces_distinct(
        logical_primary_b.as_str(),
        &provider_primary_b,
        cloud_primary.as_str(),
        "replica-B paid content-revision-2 conversation",
    );
    assert_live_text_transcript(
        &claude_home_b,
        &project_b,
        &provider_primary_b,
        &r1_history,
        2,
    );
    let destination_r1_bytes = transcript_bytes(&claude_home_b, &project_b, &provider_primary_b);
    assert_eq!(
        lineage_of(&destination_r1_bytes),
        source_r1_lineage,
        "production paid Claude relocation changed lineage or semantics"
    );
    assert_eq!(
        transcript_bytes(&claude_home_a, &project_a, LIVE_TUZI_SOURCE_SESSION_ID,),
        source_r1_bytes,
        "replica-B paid Pull mutated replica A's authoritative transcript"
    );
    reported_cost_usd += run_live_tuzi_claude(
        &mut process_count,
        &provider,
        &claude_home_b,
        &process_home,
        &project_b,
        &provider_primary_b,
        false,
        "MALLARD_CLAUDE_LIVE_USER_R2",
        "MALLARD_CLAUDE_LIVE_REMOTE_BRANCH",
    );
    let mut r2_history = r1_history.clone();
    r2_history.extend(marker_turn(
        "MALLARD_CLAUDE_LIVE_USER_R2",
        "MALLARD_CLAUDE_LIVE_REMOTE_BRANCH",
    ));
    assert_live_text_transcript(
        &claude_home_b,
        &project_b,
        &provider_primary_b,
        &r2_history,
        4,
    );
    let destination_r2_bytes = transcript_bytes(&claude_home_b, &project_b, &provider_primary_b);
    let r2_resource = mallard_domain::conversation_resource_id(&claude_key(), &logical_primary_b)
        .expect("derive paid content-revision-3 Claude resource ID");
    session_detail_text_history(
        &service_b,
        &project_id_b,
        &r2_resource,
        &provider_primary_b,
        &r2_history,
    );
    let second_push = push(&service_b, &project_id_b, &storage_id);
    assert_eq!(second_push.revision, Some(3));

    // A independently resumes its stale base. Push must be blocked, the
    // divergent Pull must offer exactly one reviewed Keep Both, and apply
    // must preserve A's paid local continuation byte-for-byte.
    reported_cost_usd += run_live_tuzi_claude(
        &mut process_count,
        &provider,
        &claude_home_a,
        &process_home,
        &project_a,
        LIVE_TUZI_SOURCE_SESSION_ID,
        false,
        "MALLARD_CLAUDE_LIVE_USER_R3",
        "MALLARD_CLAUDE_LIVE_LOCAL_BRANCH",
    );
    let mut r3_history = r1_history.clone();
    r3_history.extend(marker_turn(
        "MALLARD_CLAUDE_LIVE_USER_R3",
        "MALLARD_CLAUDE_LIVE_LOCAL_BRANCH",
    ));
    assert_live_text_transcript(
        &claude_home_a,
        &project_a,
        LIVE_TUZI_SOURCE_SESSION_ID,
        &r3_history,
        4,
    );
    let local_r3_before_keep =
        transcript_bytes(&claude_home_a, &project_a, LIVE_TUZI_SOURCE_SESSION_ID);
    assert_divergent_push_gate(
        &service_a,
        &project_id_a,
        &storage_id,
        &cloud_primary,
        "paid replica A",
    );
    let divergent_plan = redacted(
        service_a.plan_bundle_restore(&storage_id, &bundle_id, &binding_a),
        "cannot plan paid divergent Claude Pull",
    );
    assert_keep_both_offered(&divergent_plan, &logical_primary_a);
    let divergent_apply = redacted(
        service_a.apply_bundle_restore(&divergent_plan, approved_pull_actions(&divergent_plan)),
        "cannot apply paid divergent Claude Keep Both",
    );
    assert!(divergent_apply.success && divergent_apply.failed_actions.is_empty());
    let keep = assert_keep_both_result(
        &repository_a,
        &project_id_a,
        &logical_primary_a,
        &cloud_primary,
        LIVE_TUZI_SOURCE_SESSION_ID,
        &divergent_apply,
    );
    assert_uuid_v4(
        &keep.remote_provider_thread_id,
        "paid Keep Both remote Claude provider ID",
    );
    assert_eq!(
        transcript_bytes(&claude_home_a, &project_a, LIVE_TUZI_SOURCE_SESSION_ID,),
        local_r3_before_keep,
        "paid Keep Both rewrote the preserved local Claude transcript"
    );
    assert_eq!(
        root_level_transcript_stems(&claude_home_a, &project_a),
        BTreeSet::from([
            LIVE_TUZI_SOURCE_SESSION_ID.to_string(),
            keep.remote_provider_thread_id.clone(),
        ])
    );
    assert_live_text_transcript(
        &claude_home_a,
        &project_a,
        LIVE_TUZI_SOURCE_SESSION_ID,
        &r3_history,
        4,
    );
    assert_live_text_transcript(
        &claude_home_a,
        &project_a,
        &keep.remote_provider_thread_id,
        &r2_history,
        4,
    );
    assert_eq!(
        lineage_of(&transcript_bytes(
            &claude_home_a,
            &project_a,
            &keep.remote_provider_thread_id,
        )),
        lineage_of(&destination_r2_bytes),
        "paid Keep Both changed the remote lineage"
    );
    let tracking_resource =
        mallard_domain::conversation_resource_id(&claude_key(), &logical_primary_a)
            .expect("derive paid tracking resource ID");
    session_detail_text_history(
        &service_a,
        &project_id_a,
        &tracking_resource,
        &keep.remote_provider_thread_id,
        &r2_history,
    );
    let preserved_resource =
        mallard_domain::conversation_resource_id(&claude_key(), &keep.preserved_thread_id)
            .expect("derive paid preserved resource ID");
    session_detail_text_history(
        &service_a,
        &project_id_a,
        &preserved_resource,
        LIVE_TUZI_SOURCE_SESSION_ID,
        &r3_history,
    );

    // Content revision 4 publishes the two lineages. B pulls the added preserved
    // lineage without replacing its stable provider identity, then both
    // replicas prove reviewed Pull and Push fixed points.
    let third_push = push(&service_a, &project_id_a, &storage_id);
    assert_eq!(third_push.revision, Some(4));
    let preserved_cloud =
        cloud_thread_id_of(&repository_a, &project_id_a, &keep.preserved_thread_id);
    assert_ne!(preserved_cloud, cloud_primary);
    let (_, third_pull) = pull_and_apply(&service_b, &storage_id, &bundle_id, &binding_b);
    assert!(third_pull.preserved_threads.is_empty());
    assert_eq!(
        provider_thread_id(&repository_b, &project_id_b, &logical_primary_b),
        provider_primary_b,
        "paid MaterializeCloud replaced B's stable provider ID"
    );
    assert_eq!(
        transcript_bytes(&claude_home_b, &project_b, &provider_primary_b),
        destination_r2_bytes,
        "content-revision-4 Pull rewrote B's primary paid transcript"
    );
    let preserved_local_b = local_thread_of_cloud(&repository_b, &project_id_b, &preserved_cloud);
    let preserved_provider_b = provider_thread_id(&repository_b, &project_id_b, &preserved_local_b);
    assert_uuid_v4(
        &preserved_provider_b,
        "replica-B paid preserved-lineage provider ID",
    );
    assert_live_text_transcript(
        &claude_home_b,
        &project_b,
        &preserved_provider_b,
        &r3_history,
        4,
    );
    let preserved_b_resource =
        mallard_domain::conversation_resource_id(&claude_key(), &preserved_local_b)
            .expect("derive replica-B paid preserved resource ID");
    session_detail_text_history(
        &service_b,
        &project_id_b,
        &preserved_b_resource,
        &preserved_provider_b,
        &r3_history,
    );

    let expected_cloud_ids = [&cloud_primary, &preserved_cloud]
        .into_iter()
        .map(|cloud_id| cloud_id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(expected_cloud_ids.len(), 2);
    let storage_before_noops = tree_digest_inventory(&storage_root);
    assert_pull_is_no_op(&service_a, &storage_id, &bundle_id, &binding_a, 4);
    assert_pull_is_no_op(&service_b, &storage_id, &bundle_id, &binding_b, 4);
    for (service, project_id) in [(&service_a, &project_id_a), (&service_b, &project_id_b)] {
        let no_op = push(service, project_id, &storage_id);
        assert_eq!(no_op.revision, Some(4));
        assert_eq!(no_op.resources_changed, Some(0));
    }
    assert_eq!(tree_digest_inventory(&storage_root), storage_before_noops);
    assert_eq!(
        redacted(
            service_a.inspect_bundle(&storage_id, &bundle_id),
            "cannot inspect final paid Claude bundle",
        )
        .head
        .revision,
        4
    );
    let topology_a = assert_exact_replica_topology(
        &repository_a,
        &service_a,
        &binding_a,
        &project_id_a,
        &storage_id,
        &claude_home_a,
        &project_a,
        &expected_cloud_ids,
        4,
        "paid replica A",
    );
    let topology_b = assert_exact_replica_topology(
        &repository_b,
        &service_b,
        &binding_b,
        &project_id_b,
        &storage_id,
        &claude_home_b,
        &project_b,
        &expected_cloud_ids,
        4,
        "paid replica B",
    );
    assert_cross_replica_topology(&topology_a, &topology_b, &expected_cloud_ids);

    eprintln!("live paid Claude phase: storage reset and session republish");
    let claude_home_a_before_reset = tree_digest_inventory(&claude_home_a);
    let claude_home_b_before_reset = tree_digest_inventory(&claude_home_b);
    let project_before_reset = redacted(
        service_a.inspect_bundle(&storage_id, &bundle_id),
        "cannot inspect paid Claude bundle before storage reset",
    );
    let conversations_before_reset = project_before_reset
        .manifest
        .all_conversations()
        .map(|(_, cloud_id, thread)| {
            (
                cloud_id.clone(),
                (
                    thread.provider.clone(),
                    thread.lineage_digest.clone(),
                    thread.semantic_digest.clone(),
                    thread.message_count,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(conversations_before_reset.len(), 2);

    let reset_a = clear_storage_for_replica(&repository_a, &service_a, &storage_id);
    assert!(reset_a.repository_objects_deleted > 0);
    assert_eq!(reset_a.library_objects_deleted, 0);
    assert_eq!(reset_a.project_links_reset, 1);
    assert_eq!(reset_a.catalog_links_reset, 0);
    let reset_b = clear_storage_for_replica(&repository_b, &service_b, &storage_id);
    assert_eq!(reset_b.repository_objects_deleted, 0);
    assert_eq!(reset_b.library_objects_deleted, 0);
    assert_eq!(reset_b.project_links_reset, 1);
    assert_eq!(reset_b.catalog_links_reset, 0);
    assert!(service_a.inspect_bundle(&storage_id, &bundle_id).is_err());
    assert_eq!(
        tree_digest_inventory(&claude_home_a),
        claude_home_a_before_reset
    );
    assert_eq!(
        tree_digest_inventory(&claude_home_b),
        claude_home_b_before_reset
    );

    let reset_push = push(&service_a, &project_id_a, &storage_id);
    assert!(reset_push.success, "{}", reset_push.message);
    assert_eq!(reset_push.revision, Some(1));
    let (_, reset_pull) = pull_and_apply(&service_b, &storage_id, &bundle_id, &binding_b);
    assert!(reset_pull.preserved_threads.is_empty());
    let project_after_reset = redacted(
        service_a.inspect_bundle(&storage_id, &bundle_id),
        "cannot inspect republished paid Claude bundle",
    );
    assert_eq!(project_after_reset.head.revision, 1);
    let conversations_after_reset = project_after_reset
        .manifest
        .all_conversations()
        .map(|(_, cloud_id, thread)| {
            (
                cloud_id.clone(),
                (
                    thread.provider.clone(),
                    thread.lineage_digest.clone(),
                    thread.semantic_digest.clone(),
                    thread.message_count,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(conversations_after_reset, conversations_before_reset);
    let reset_topology_a = assert_exact_replica_topology(
        &repository_a,
        &service_a,
        &binding_a,
        &project_id_a,
        &storage_id,
        &claude_home_a,
        &project_a,
        &expected_cloud_ids,
        1,
        "paid replica A after storage reset",
    );
    let reset_topology_b = assert_exact_replica_topology(
        &repository_b,
        &service_b,
        &binding_b,
        &project_id_b,
        &storage_id,
        &claude_home_b,
        &project_b,
        &expected_cloud_ids,
        1,
        "paid replica B after storage reset",
    );
    assert_eq!(reset_topology_a.by_cloud, topology_a.by_cloud);
    assert_eq!(reset_topology_b.by_cloud, topology_b.by_cloud);
    assert_cross_replica_topology(&reset_topology_a, &reset_topology_b, &expected_cloud_ids);
    assert_eq!(
        tree_digest_inventory(&claude_home_a),
        claude_home_a_before_reset
    );
    assert_eq!(
        tree_digest_inventory(&claude_home_b),
        claude_home_b_before_reset
    );

    assert_eq!(
        process_count, EXPECTED_LIVE_TUZI_PROCESSES,
        "paid Claude process budget changed"
    );
    assert!(
        reported_cost_usd.is_finite()
            && (0.0..=MAX_LIVE_TUZI_COST_PER_TURN_USD * EXPECTED_LIVE_TUZI_PROCESSES as f64)
                .contains(&reported_cost_usd),
        "paid Claude aggregate client estimate exceeded its process budget"
    );

    let mut final_machine_identity_needles = identity_needles([
        project_id_a.as_str(),
        project_id_b.as_str(),
        logical_primary_a.as_str(),
        logical_primary_b.as_str(),
        keep.preserved_thread_id.as_str(),
        preserved_local_b.as_str(),
        storage_id.as_str(),
        binding_a.host_id.as_str(),
        binding_a.replica_id.as_str(),
        binding_b.host_id.as_str(),
        binding_b.replica_id.as_str(),
    ]);
    final_machine_identity_needles.extend(
        binding_a
            .profile_ids
            .values()
            .chain(binding_b.profile_ids.values())
            .map(|profile_id| profile_id.as_str().as_bytes().to_vec()),
    );
    final_machine_identity_needles.sort();
    final_machine_identity_needles.dedup();
    assert_portable_state_is_safe(
        &service_a,
        &storage_id,
        &bundle_id,
        &storage_root,
        &sensitive,
        &path_needles,
        &final_machine_identity_needles,
    );
    for (root, label) in [
        (&claude_home_a, "paid replica-A Claude home"),
        (&claude_home_b, "paid replica-B Claude home"),
        (&machine_a_home, "paid replica-A Mallard home"),
        (&machine_b_home, "paid replica-B Mallard home"),
        (&process_home, "paid Claude process home"),
    ] {
        assert_private_tree(root, &sensitive, label);
    }
}
