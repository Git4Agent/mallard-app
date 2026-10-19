//! Opt-in end-to-end coverage for two isolated Codex homes synchronizing
//! through one real Mallard local-folder repository.
//!
//! This test makes five paid, authenticated Codex CLI turns to create source
//! conversations, then reopens twenty synchronized histories through public
//! headless Codex CLI resumes against a bounded loopback Responses recorder.
//! Between those phases, it clears and republishes the shared storage without
//! changing either replica's conversation content or titles.
//! It is ignored by default, separately environment-gated, and intended to run
//! as the only test process so its temporary `HOME` override cannot affect
//! other tests.

#[path = "support/codex_home.rs"]
mod codex_home;
#[path = "support/codex_output.rs"]
mod codex_output;
#[path = "support/session_reconciliation.rs"]
mod session_reconciliation;
mod support;

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use mallard_application::project_sync::service::{
    ProjectDetail, ProjectOperationResult, ProjectSessionTimelineRequest, ProjectSyncService,
    PushApproval, PushDestinationPullReason, PushDestinationStatus, RestoreResult,
    SessionTimelineEventKind, SessionToolCategory, StorageDataClearResult, ThreadSyncComparison,
    ThreadSyncState,
};
use mallard_domain::{
    ActionId, BundleId, CloudThreadId, ConversationFileRole, ConversationReviewAction,
    DraftProfileSelection, DraftRepositoryChoice, DraftStorageSelection, LocalConversationBinding,
    LocalProjectId, ProjectBinding, ProjectSetupDraft, ProviderKey, ResourceId, RestoreActionKind,
    RestorePlan, StorageConfig, StorageId, SyncScopeKey, ThreadId, MAX_CONVERSATION_FILE_BYTES,
};
use mallard_infrastructure::codex_history_graph::{resolve_codex_root, seed_codex_root};
use mallard_infrastructure::codex_paginated_projection::verify_projection_fixpoint;
use mallard_infrastructure::codex_rollout::analyze_codex_rollout_structure;
use mallard_infrastructure::codex_standalone_builder::{
    decode_standalone_codex_root, encode_standalone_codex_root,
};
use mallard_infrastructure::local_paths::canonicalize;
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::providers::{
    AgentProviderRegistry, SessionCountQuality, SessionTimelineContext, SessionTimelineQuery,
};
use mallard_infrastructure::storage_core::repository_object_key;
use mallard_infrastructure::storage_providers::local::local_storage_config;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime};
use walkdir::WalkDir;

use support::catalog_flow::{run_catalog_flow, storage_snapshot, CatalogFlowInputs};

const LIVE_TEST_GATE: &str = "MALLARD_RUN_LIVE_CODEX_E2E";
const MODEL: &str = "gpt-6-luna";
const PROVIDER_ID: &str = "mallard_live";
const PROVIDER_KEY_ENV: &str = "MALLARD_CODEX_PROVIDER_API_KEY";
const HEADLESS_REOPEN_PROMPT: &str =
    "HEADLESS_REOPEN_PROBE_USER Verify the restored history transport without using tools.";
const HEADLESS_REOPEN_ACK: &str = "MALLARD_HEADLESS_REOPEN_CAPTURED";
const CREDENTIAL_ENV_KEYS: [&str; 7] = [
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "MALLARD_LIVE_CODEX_PROVIDER_KEY",
    "MALLARD_LIVE_CODEX_PROVIDER_URL",
    "MALLARD_TUZI_POC_KEY",
    PROVIDER_KEY_ENV,
];
const PROXY_ENV_KEYS: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];
const REASONING_OVERRIDE: &str = "model_reasoning_effort=\"low\"";
const MAX_PROVIDER_DOCUMENT_BYTES: u64 = 4 * 1024;
const MAX_PROVIDER_KEY_BYTES: usize = 1_024;
const MAX_CODEX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_CODEX_OUTPUT_EVENTS: usize = 4_096;
const MAX_CODEX_SYNOPSIS_EVENTS: usize = 64;
const MAX_COMMAND_ARGUMENT_BYTES: usize = 4096;
const MAX_COMMAND_ARGUMENTS: usize = 48;
const MAX_PROMPT_BYTES: usize = 512;
const MAX_WRITE_PROMPT_BYTES: usize = 2 * 1024;
const MAX_STORAGE_OBJECT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SNAPSHOT_FILES: usize = 2_048;
const MAX_FIXTURE_FILE_BYTES: u64 = 4 * 1024;
const MAX_FIXTURE_TREE_ENTRIES: usize = 32;
const MAX_SESSION_DETAIL_EVENTS: usize = 1_024;
const HISTORY_TIMELINE_PAGE_LIMIT: usize = 2;
const MAX_CODEX_HOME_PRIVACY_ENTRIES: usize = 12_000;
const MAX_CODEX_HOME_PRIVACY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_CAPTURE_REQUEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_CAPTURE_HEADERS: usize = 64;
const MAX_CAPTURE_HEADER_BYTES: usize = 32 * 1024;
const MAX_CAPTURE_INPUT_ITEMS: usize = 4_096;
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
const CODEX_TIMEOUT: Duration = Duration::from_secs(180);
const EXPECTED_CODEX_VERSION: &str = "codex-cli 0.162.1";
const EXPECTED_PAID_CODEX_TURNS: usize = 5;
const EXPECTED_HEADLESS_RESUMES: usize = 20;
const EXPECTED_CODEX_CLI_TURNS: usize = EXPECTED_PAID_CODEX_TURNS + EXPECTED_HEADLESS_RESUMES;

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
        mallard_infrastructure::providers::codex::cli::resolve(None).map(|cli| cli.executable),
        "the live test requires an installed Codex CLI",
    )
}

fn assert_supported_codex_version() {
    let isolated = tempfile::tempdir().expect("create isolated Codex version-probe root");
    let process_home = isolated.path().join("process-home");
    let codex_home = isolated.path().join("codex-home");
    std::fs::create_dir(&process_home).expect("create isolated Codex version-probe home");
    create_secure_codex_home(&codex_home);

    let mut command = Command::new(codex_binary());
    remove_credential_environment(&mut command);
    let output = command
        .arg("--version")
        .env("CODEX_HOME", &codex_home)
        .env("HOME", &process_home)
        .env("USERPROFILE", &process_home)
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
    assert_eq!(
        version.trim(),
        EXPECTED_CODEX_VERSION,
        "the live fixture requires Codex 0.162.1; update the current-format writer before running against another version"
    );
    assert_codex_home_has_no_sensitive_state(&codex_home, &[]);
    assert_process_home_has_no_sensitive_state(&process_home, &[]);
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveProviderDocument {
    #[serde(rename = "_type")]
    connection_type: String,
    key: String,
    url: String,
}

struct LiveProviderConnection {
    api_key: String,
    base_url: String,
}

impl LiveProviderConnection {
    fn credential_needles(&self) -> Vec<Vec<u8>> {
        credential_needles(self.api_key.as_bytes())
    }

    fn config_arguments(&self) -> Vec<OsString> {
        // This fixture exercises Mallard skill/pet portability, not the Codex
        // curated marketplace. Disable its background repository sync so
        // repeated short-lived CLIs cannot race or retain plugin clones in
        // the otherwise bounded provider homes.
        let overrides = [
            format!("model_provider=\"{PROVIDER_ID}\""),
            REASONING_OVERRIDE.to_string(),
            "features.plugins=false".to_string(),
            format!("model_providers.{PROVIDER_ID}.name=\"Mallard live Responses provider\""),
            format!(
                "model_providers.{PROVIDER_ID}.base_url={}",
                serde_json::to_string(&self.base_url).expect("serialize validated provider URL")
            ),
            format!("model_providers.{PROVIDER_ID}.env_key=\"{PROVIDER_KEY_ENV}\""),
            format!("model_providers.{PROVIDER_ID}.wire_api=\"responses\""),
            format!("model_providers.{PROVIDER_ID}.requires_openai_auth=false"),
            format!("model_providers.{PROVIDER_ID}.request_max_retries=0"),
            format!("model_providers.{PROVIDER_ID}.stream_max_retries=0"),
            format!("model_providers.{PROVIDER_ID}.stream_idle_timeout_ms=60000"),
        ];
        let mut arguments = Vec::with_capacity(overrides.len() * 2);
        for value in overrides {
            arguments.push(OsString::from("--config"));
            arguments.push(OsString::from(value));
        }
        arguments
    }
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

fn normalize_provider_url(value: &str) -> Result<String, &'static str> {
    if value.is_empty()
        || value.len() > 2_048
        || value.chars().any(char::is_control)
        || value.trim() != value
        || value.contains('@')
    {
        return Err("invalid_provider_url");
    }
    let mut url = tauri::Url::parse(value).map_err(|_| "invalid_provider_url")?;
    if url.scheme() != "https"
        || url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "/" | "/v1")
    {
        return Err("invalid_provider_url");
    }
    url.set_path("/v1");
    Ok(url.to_string().trim_end_matches('/').to_string())
}

fn parse_live_provider_connection(bytes: &[u8]) -> Result<LiveProviderConnection, &'static str> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_PROVIDER_DOCUMENT_BYTES {
        return Err("invalid_provider_document");
    }
    let document: LiveProviderDocument =
        serde_json::from_slice(bytes).map_err(|_| "invalid_provider_document")?;
    if document.connection_type != "newapi_channel_conn"
        || document.key.is_empty()
        || document.key.len() > MAX_PROVIDER_KEY_BYTES
        || document.key.chars().any(char::is_control)
        || document.key.trim() != document.key
    {
        return Err("invalid_provider_document");
    }
    Ok(LiveProviderConnection {
        api_key: document.key,
        base_url: normalize_provider_url(&document.url)?,
    })
}

fn read_provider_document(path: &Path) -> Vec<u8> {
    let metadata =
        std::fs::symlink_metadata(path).expect("live provider configuration is unavailable");
    assert!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "live provider configuration must be a no-follow regular file"
    );
    assert!(
        metadata.len() <= MAX_PROVIDER_DOCUMENT_BYTES,
        "live provider configuration exceeds its safety bound"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
        assert_eq!(
            metadata.mode() & 0o7777,
            0o600,
            "live provider configuration must have Unix mode 0600"
        );
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .expect("live provider configuration is unreadable");
        let opened = file
            .metadata()
            .expect("inspect opened live provider configuration");
        assert!(
            opened.is_file()
                && opened.dev() == metadata.dev()
                && opened.ino() == metadata.ino()
                && opened.len() == metadata.len(),
            "live provider configuration changed while opening"
        );
        assert_eq!(
            opened.mode() & 0o7777,
            0o600,
            "live provider configuration permissions changed while opening"
        );
        let mut bytes = Vec::with_capacity(opened.len() as usize);
        file.take(MAX_PROVIDER_DOCUMENT_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .expect("live provider configuration is unreadable");
        assert!(
            bytes.len() as u64 <= MAX_PROVIDER_DOCUMENT_BYTES,
            "live provider configuration exceeds its safety bound"
        );
        bytes
    }
    #[cfg(not(unix))]
    read_bounded_file(
        path,
        MAX_PROVIDER_DOCUMENT_BYTES,
        "live provider configuration",
    )
}

fn load_live_provider_connection() -> (PathBuf, LiveProviderConnection) {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository_root = manifest_dir
        .parent()
        .expect("Tauri manifest directory must have a repository parent");
    let path = repository_root.join(".env.local");
    let bytes = read_provider_document(&path);
    let connection = parse_live_provider_connection(&bytes)
        .expect("live provider configuration is missing or invalid");
    let canonical_path =
        canonicalize(path).expect("canonicalize live provider configuration");
    (canonical_path, connection)
}

fn create_secure_codex_home(path: &Path) {
    std::fs::create_dir(path).expect("create isolated Codex home");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .expect("secure isolated Codex home");
    }
    assert!(!path.join("auth.json").exists());
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
    let canonical_root = canonicalize(root).expect("canonicalize live path fixture root");
    for (relative, _) in fixtures {
        let relative = clean_fixture_relative_path(relative);
        let parent = relative.parent().expect("fixture path has a parent");
        std::fs::create_dir_all(root.join(parent)).expect("create live path fixture parent");
        assert_no_follow_directory_chain(root, parent);
        let canonical_parent =
            canonicalize(root.join(parent)).expect("canonicalize fixture parent");
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
    let source_root = canonicalize(source_root).expect("canonicalize source project");
    let target_root = canonicalize(target_root).expect("canonicalize target project");
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
            canonicalize(target_parent).expect("canonicalize copied fixture parent");
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
    let root = canonicalize(root).expect("canonicalize live path fixture root");
    fixtures
        .iter()
        .map(|(relative, _)| root.join(clean_fixture_relative_path(relative)))
        .collect()
}

fn fixture_path_spellings(root: &Path, relative: &str) -> BTreeSet<PathBuf> {
    let relative = clean_fixture_relative_path(relative);
    let canonical_root =
        canonicalize(root).expect("canonicalize live path fixture root spelling");
    [root.join(relative), canonical_root.join(relative)]
        .into_iter()
        .collect()
}

fn revision_one_patch_body(outside_root: &Path) -> String {
    assert!(
        outside_root.is_absolute(),
        "outside fixture patch root must be absolute"
    );
    let outside_root = outside_root
        .to_str()
        .expect("outside fixture patch root must be UTF-8");
    let mut patch = String::from("*** Begin Patch\n");
    for (relative, contents) in IN_PROJECT_FIXTURES {
        let contents = std::str::from_utf8(contents).expect("fixture bytes must be UTF-8");
        let contents = contents
            .strip_suffix('\n')
            .expect("fixture bytes must end in one newline");
        assert!(
            !contents.contains('\n'),
            "revision-1 patch fixtures must each contain one line"
        );
        patch.push_str(&format!("*** Add File: ./{relative}\n+{contents}\n"));
    }
    for (relative, contents) in OUTSIDE_PROJECT_FIXTURES {
        let contents = std::str::from_utf8(contents).expect("fixture bytes must be UTF-8");
        let contents = contents
            .strip_suffix('\n')
            .expect("fixture bytes must end in one newline");
        assert!(
            !contents.contains('\n'),
            "revision-1 patch fixtures must each contain one line"
        );
        patch.push_str(&format!(
            "*** Add File: {outside_root}/{relative}\n+{contents}\n"
        ));
    }
    patch.push_str("*** End Patch\n");
    patch
}

fn revision_one_patch_command(outside_root: &Path) -> String {
    let command = format!(
        "apply_patch <<'MALLARD_PATCH'\n{}MALLARD_PATCH\n",
        revision_one_patch_body(outside_root)
    );
    assert!(
        command.len() <= MAX_COMMAND_ARGUMENT_BYTES
            && !command.contains('`')
            && !command.contains("$("),
        "revision-1 deterministic patch command violates its safety bound"
    );
    command
}

fn revision_one_file_prompt(project_root: &Path, outside_root: &Path) -> String {
    assert!(
        project_root.is_absolute() && outside_root.is_absolute(),
        "prompt fixture roots must be absolute"
    );
    let workdir = project_root
        .to_str()
        .expect("live path fixture must have a UTF-8 project root");
    let arguments = json!({
        "cmd": revision_one_patch_command(outside_root),
        "workdir": workdir,
    });
    let exact_arguments =
        serde_json::to_string(&arguments).expect("serialize revision-1 exec arguments");
    let prompt = format!(
        "LIVE_USER_R1 Make exactly one exec_command tool call with exactly these JSON arguments, copied byte-for-byte without rewriting the command, workdir, patch paths, or content: {exact_arguments}. Do not use code mode or invoke any other tool. Only after the command succeeds, reply with exactly: MALLARD_BASE",
    );
    assert!(
        prompt.len() <= MAX_WRITE_PROMPT_BYTES && !prompt.chars().any(char::is_control),
        "revision-1 file prompt violates the argument bound"
    );
    prompt
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

fn remove_credential_environment(command: &mut Command) {
    for key in CREDENTIAL_ENV_KEYS {
        command.env_remove(key);
    }
}

fn configure_codex_child(command: &mut Command, provider: &LiveProviderConnection) {
    remove_credential_environment(command);
    command.env(PROVIDER_KEY_ENV, &provider.api_key);
}

fn capture_provider_config_arguments(base_url: &str) -> Vec<OsString> {
    assert!(
        base_url.starts_with("http://127.0.0.1:") && base_url.ends_with("/v1"),
        "headless capture provider must use literal IPv4 loopback"
    );
    let overrides = [
        format!("model_provider=\"{PROVIDER_ID}\""),
        REASONING_OVERRIDE.to_string(),
        "features.plugins=false".to_string(),
        "features.enable_request_compression=false".to_string(),
        format!("model_providers.{PROVIDER_ID}.name=\"Mallard loopback Responses capture\""),
        format!(
            "model_providers.{PROVIDER_ID}.base_url={}",
            serde_json::to_string(base_url).expect("serialize loopback provider URL")
        ),
        format!("model_providers.{PROVIDER_ID}.wire_api=\"responses\""),
        format!("model_providers.{PROVIDER_ID}.requires_openai_auth=false"),
        format!("model_providers.{PROVIDER_ID}.request_max_retries=0"),
        format!("model_providers.{PROVIDER_ID}.stream_max_retries=0"),
        format!("model_providers.{PROVIDER_ID}.stream_idle_timeout_ms=60000"),
    ];
    let mut arguments = Vec::with_capacity(overrides.len() * 2);
    for value in overrides {
        arguments.push(OsString::from("--config"));
        arguments.push(OsString::from(value));
    }
    arguments
}

fn configure_codex_capture_child(command: &mut Command) {
    remove_credential_environment(command);
    for key in PROXY_ENV_KEYS {
        command.env_remove(key);
    }
    command
        .env("NO_PROXY", "127.0.0.1")
        .env("no_proxy", "127.0.0.1");
}

#[allow(clippy::too_many_arguments)]
fn run_codex(
    codex_turn_count: &mut usize,
    provider: &LiveProviderConnection,
    codex_home: &Path,
    process_home: &Path,
    project_root: &Path,
    access: CodexRunAccess<'_>,
    resume_thread_id: Option<&str>,
    prompt: &str,
    expected_final_marker: &str,
) -> String {
    let prompt_limit = match access {
        CodexRunAccess::ReadOnly => MAX_PROMPT_BYTES,
        CodexRunAccess::WorkspaceWrite { .. } => MAX_WRITE_PROMPT_BYTES,
    };
    assert!(
        !prompt.is_empty() && prompt.len() <= prompt_limit && !prompt.chars().any(char::is_control),
        "live Codex prompt violates the test argument bound"
    );
    assert!(
        !expected_final_marker.is_empty()
            && expected_final_marker.len() <= 128
            && expected_final_marker
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'),
        "live Codex expected marker violates the test argument bound"
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
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    arguments.extend(provider.config_arguments());
    arguments.push(OsString::from("--cd"));
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
                canonicalize(project_root).expect("canonicalize live Codex project");
            let canonical_additional = canonicalize(additional_dir)
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
    let credential_needles = provider.credential_needles();
    for argument in &arguments {
        assert_no_sensitive_bytes(
            argument.as_encoded_bytes(),
            &credential_needles,
            "live Codex command argument",
        );
    }

    let mut command = Command::new(codex_binary());
    configure_codex_child(&mut command, provider);
    let mut child = command
        .args(arguments)
        .current_dir(project_root)
        .env("CODEX_HOME", codex_home)
        .env("HOME", process_home)
        .env("USERPROFILE", process_home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the Codex CLI");
    *codex_turn_count = codex_turn_count.saturating_add(1);
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
    assert_no_sensitive_bytes(&stdout, &credential_needles, "live Codex stdout");
    assert_no_sensitive_bytes(&stderr, &credential_needles, "live Codex stderr");
    assert_codex_home_has_no_sensitive_state(codex_home, &credential_needles);
    assert_process_home_has_no_sensitive_state(process_home, &credential_needles);
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
    assert_rollout_provider(codex_home, &provider_thread_id);

    let final_messages = events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("item.completed"))
        .filter_map(|event| event.get("item"))
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("agent_message"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(
        final_messages.len() == 1 && final_messages[0] == expected_final_marker,
        "live Codex output lacked its single exact final marker; events: {event_synopsis}; stderr: {stderr_synopsis}"
    );

    if matches!(access, CodexRunAccess::WorkspaceWrite { .. }) {
        let completed_file_changes = events
            .iter()
            .filter(|event| event.get("type").and_then(Value::as_str) == Some("item.completed"))
            .filter_map(|event| event.get("item"))
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("file_change"))
            .count();
        assert_eq!(
            completed_file_changes, 1,
            "revision-1 Codex output lacked exactly one completed file-change item; events: {event_synopsis}; stderr: {stderr_synopsis}"
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
        let file_change_keys = try_file_change_keys(&rollout).unwrap_or_else(|reason| {
            panic!(
                "revision-1 file-change evidence failed ({reason}); rollout: {}; events: {event_synopsis}; stderr: {stderr_synopsis}",
                rollout_shape_synopsis(&rollout),
            )
        });
        assert!(
            local_file_change_key_candidates(
                project_root,
                match access {
                    CodexRunAccess::WorkspaceWrite { additional_dir } => additional_dir,
                    CodexRunAccess::ReadOnly => unreachable!(),
                },
            )
            .iter()
            .any(|expected| expected == &file_change_keys),
            "revision-1 file-change keys did not exactly match the requested fixtures; events: {event_synopsis}; stderr: {stderr_synopsis}"
        );
    }

    provider_thread_id
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum HistoryRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HistoryMarker {
    role: HistoryRole,
    marker: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MarkerPrefixError {
    raw_marker_bytes: usize,
    prefix_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimelineHistoryError {
    error_code: usize,
    entry_index: usize,
    detail_a: usize,
    detail_b: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HistoryComparison {
    expected_items: usize,
    actual_items: usize,
    first_mismatch_index: usize,
    role_mismatches: usize,
    marker_mismatches: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CaptureError {
    error_code: usize,
    detail_a: usize,
    detail_b: usize,
}

struct CapturedResponsesRequest {
    method: Method,
    path: String,
    has_query: bool,
    content_encoding_present: bool,
    content_type_is_json: bool,
    authorization_present: bool,
    input_items: usize,
    history_items: usize,
    probe_input_index: usize,
}

struct OneShotResponsesCapture {
    base_url: String,
    receiver: mpsc::Receiver<Result<CapturedResponsesRequest, CaptureError>>,
    request_count: Arc<AtomicUsize>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    server: Option<std::thread::JoinHandle<Result<(), CaptureError>>>,
}

impl Drop for OneShotResponsesCapture {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

fn capture_http_response(
    status: StatusCode,
    body: Bytes,
    content_type: &str,
) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("connection", "close")
        .body(Full::new(body))
        .expect("build bounded loopback capture response")
}

fn capture_error_response() -> Response<Full<Bytes>> {
    capture_http_response(
        StatusCode::BAD_REQUEST,
        Bytes::from_static(b"invalid loopback capture request"),
        "text/plain",
    )
}

fn capture_sse_response() -> Response<Full<Bytes>> {
    let response_id = "resp-mallard-headless-capture";
    let message_id = "msg-mallard-headless-capture";
    let events = [
        json!({
            "type": "response.created",
            "response": {"id": response_id},
        }),
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "message",
                "role": "assistant",
                "id": message_id,
                "content": [{"type": "output_text", "text": HEADLESS_REOPEN_ACK}],
            },
        }),
        json!({
            "type": "response.completed",
            "response": {
                "id": response_id,
                "usage": {
                    "input_tokens": 0,
                    "input_tokens_details": null,
                    "output_tokens": 0,
                    "output_tokens_details": null,
                    "total_tokens": 0,
                },
            },
        }),
    ];
    let mut body = String::new();
    for event in events {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .expect("capture SSE event has a type");
        body.push_str("event: ");
        body.push_str(kind);
        body.push_str("\ndata: ");
        body.push_str(&event.to_string());
        body.push_str("\n\n");
    }
    capture_http_response(StatusCode::OK, Bytes::from(body), "text/event-stream")
}

fn send_capture_result(
    sender: &mpsc::SyncSender<Result<CapturedResponsesRequest, CaptureError>>,
    result: Result<CapturedResponsesRequest, CaptureError>,
) {
    let _ = sender.try_send(result);
}

fn validate_captured_responses_body(
    body: &Value,
    expected: &[HistoryMarker],
) -> Result<(usize, usize, usize), CaptureError> {
    let failure = |error_code, detail_a, detail_b| CaptureError {
        error_code,
        detail_a,
        detail_b,
    };
    if body.get("model").and_then(Value::as_str) != Some(MODEL) {
        return Err(failure(30, 0, 0));
    }
    if body.get("stream").and_then(Value::as_bool) != Some(true) {
        return Err(failure(31, 0, 0));
    }
    if body
        .get("reasoning")
        .and_then(|reasoning| reasoning.get("effort"))
        .and_then(Value::as_str)
        != Some("low")
    {
        return Err(failure(32, 0, 0));
    }
    let input = body
        .get("input")
        .and_then(Value::as_array)
        .ok_or_else(|| failure(33, 0, 0))?;
    if input.len() > MAX_CAPTURE_INPUT_ITEMS {
        return Err(failure(34, input.len(), MAX_CAPTURE_INPUT_ITEMS));
    }

    let mut actual = Vec::with_capacity(expected.len());
    let mut probe_count = 0usize;
    let mut probe_input_index = usize::MAX;
    for (input_index, item) in input.iter().enumerate() {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let role = item.get("role").and_then(Value::as_str);
        if !matches!(role, Some("user" | "assistant")) {
            continue;
        }
        let Some(content) = item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for span in content {
            let span_type = span.get("type").and_then(Value::as_str);
            let Some(text) = span.get("text").and_then(Value::as_str) else {
                continue;
            };
            let Ok(prefix) = normalize_position_zero_marker(text) else {
                continue;
            };
            if prefix == "HEADLESS_REOPEN_PROBE_USER" {
                if role != Some("user")
                    || span_type != Some("input_text")
                    || text != HEADLESS_REOPEN_PROMPT
                    || probe_count != 0
                {
                    return Err(failure(35, input_index, probe_count));
                }
                probe_count = probe_count.saturating_add(1);
                probe_input_index = input_index;
                continue;
            }

            let fixture_role =
                if prefix.starts_with("LIVE_USER_") || prefix.starts_with("GRAPH_USER_") {
                    Some(HistoryRole::User)
                } else if prefix.starts_with("MALLARD_")
                    || (prefix.starts_with("GRAPH_") && !prefix.starts_with("GRAPH_USER_"))
                {
                    Some(HistoryRole::Assistant)
                } else {
                    None
                };
            let Some(fixture_role) = fixture_role else {
                continue;
            };
            if probe_count != 0 {
                return Err(failure(36, input_index, actual.len()));
            }
            let marker = match fixture_role {
                HistoryRole::User => {
                    if role != Some("user") || span_type != Some("input_text") {
                        return Err(failure(37, input_index, actual.len()));
                    }
                    prefix
                }
                HistoryRole::Assistant => {
                    if role != Some("assistant") || span_type != Some("output_text") {
                        return Err(failure(38, input_index, actual.len()));
                    }
                    validate_exact_marker(text)
                        .map_err(|error| failure(39, error.raw_marker_bytes, error.prefix_bytes))?
                }
            };
            actual.push(HistoryMarker {
                role: fixture_role,
                marker,
            });
        }
    }
    if probe_count != 1 || probe_input_index == usize::MAX {
        return Err(failure(40, probe_count, input.len()));
    }
    let comparison = compare_ordered_history(&actual, expected);
    if actual != expected {
        return Err(failure(
            41,
            comparison.first_mismatch_index,
            comparison.actual_items,
        ));
    }
    Ok((input.len(), actual.len(), probe_input_index))
}

async fn handle_responses_capture(
    request: Request<Incoming>,
    sender: mpsc::SyncSender<Result<CapturedResponsesRequest, CaptureError>>,
    request_count: Arc<AtomicUsize>,
    expected: Arc<Vec<HistoryMarker>>,
    sensitive_needles: Arc<Vec<Vec<u8>>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let request_index = request_count.fetch_add(1, Ordering::SeqCst);
    if request_index != 0 {
        return Ok(capture_http_response(
            StatusCode::CONFLICT,
            Bytes::from_static(b"loopback capture accepts one request"),
            "text/plain",
        ));
    }

    let method_is_post = request.method() == Method::POST;
    let path_is_responses = request.uri().path() == "/v1/responses";
    let has_query = request.uri().query().is_some();
    if !method_is_post || !path_is_responses || has_query {
        send_capture_result(
            &sender,
            Err(CaptureError {
                error_code: 19,
                detail_a: usize::from(method_is_post),
                detail_b: usize::from(path_is_responses && !has_query),
            }),
        );
        return Ok(capture_error_response());
    }

    let header_count = request.headers().len();
    if header_count > MAX_CAPTURE_HEADERS {
        send_capture_result(
            &sender,
            Err(CaptureError {
                error_code: 20,
                detail_a: header_count,
                detail_b: MAX_CAPTURE_HEADERS,
            }),
        );
        return Ok(capture_error_response());
    }
    let header_bytes = request
        .headers()
        .iter()
        .fold(0usize, |total, (name, value)| {
            total
                .saturating_add(name.as_str().len())
                .saturating_add(value.as_bytes().len())
        });
    if header_bytes > MAX_CAPTURE_HEADER_BYTES {
        send_capture_result(
            &sender,
            Err(CaptureError {
                error_code: 21,
                detail_a: header_bytes,
                detail_b: MAX_CAPTURE_HEADER_BYTES,
            }),
        );
        return Ok(capture_error_response());
    }
    if request.headers().iter().any(|(name, value)| {
        sensitive_needles.iter().any(|needle| {
            byte_slice_contains(name.as_str().as_bytes(), needle)
                || byte_slice_contains(value.as_bytes(), needle)
        })
    }) {
        send_capture_result(
            &sender,
            Err(CaptureError {
                error_code: 22,
                detail_a: header_count,
                detail_b: header_bytes,
            }),
        );
        return Ok(capture_error_response());
    }

    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let content_encoding_present = request.headers().contains_key("content-encoding");
    let content_type_is_json = request
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|media_type| media_type.trim() == "application/json")
        });
    let authorization_present = request.headers().contains_key("authorization");
    let body = match Limited::new(request.into_body(), MAX_CAPTURE_REQUEST_BYTES)
        .collect()
        .await
    {
        Ok(body) => body.to_bytes(),
        Err(_) => {
            send_capture_result(
                &sender,
                Err(CaptureError {
                    error_code: 23,
                    detail_a: 0,
                    detail_b: MAX_CAPTURE_REQUEST_BYTES,
                }),
            );
            return Ok(capture_error_response());
        }
    };
    let body_bytes = body.len();
    if sensitive_needles
        .iter()
        .any(|needle| byte_slice_contains(&body, needle))
    {
        send_capture_result(
            &sender,
            Err(CaptureError {
                error_code: 24,
                detail_a: body_bytes,
                detail_b: sensitive_needles.len(),
            }),
        );
        return Ok(capture_error_response());
    }
    let body: Value = match serde_json::from_slice(&body) {
        Ok(body) => body,
        Err(_) => {
            send_capture_result(
                &sender,
                Err(CaptureError {
                    error_code: 25,
                    detail_a: body_bytes,
                    detail_b: 0,
                }),
            );
            return Ok(capture_error_response());
        }
    };
    let (input_items, history_items, probe_input_index) =
        match validate_captured_responses_body(&body, &expected) {
            Ok(report) => report,
            Err(error) => {
                send_capture_result(&sender, Err(error));
                return Ok(capture_error_response());
            }
        };
    send_capture_result(
        &sender,
        Ok(CapturedResponsesRequest {
            method,
            path,
            has_query,
            content_encoding_present,
            content_type_is_json,
            authorization_present,
            input_items,
            history_items,
            probe_input_index,
        }),
    );
    Ok(capture_sse_response())
}

fn start_one_shot_responses_capture(
    expected: &[HistoryMarker],
    sensitive_needles: &[Vec<u8>],
) -> OneShotResponsesCapture {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|_| panic!("cannot bind the bounded loopback Responses capture"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|_| panic!("cannot resolve the loopback Responses capture address"));
    assert!(
        address.ip() == std::net::Ipv4Addr::LOCALHOST,
        "Responses capture did not bind literal IPv4 loopback"
    );
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|_| panic!("cannot configure the loopback Responses capture"));
    let base_url = format!("http://127.0.0.1:{}/v1", address.port());
    let (sender, receiver) = mpsc::sync_channel(1);
    let request_count = Arc::new(AtomicUsize::new(0));
    let server_request_count = request_count.clone();
    let expected = Arc::new(expected.to_vec());
    let sensitive_needles = Arc::new(sensitive_needles.to_vec());
    let (cancel, mut cancel_receiver) = tokio::sync::oneshot::channel();
    let server = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| CaptureError {
                error_code: 1,
                detail_a: 0,
                detail_b: 0,
            })?;
        runtime.block_on(async move {
            let listener =
                tokio::net::TcpListener::from_std(listener).map_err(|_| CaptureError {
                    error_code: 2,
                    detail_a: 0,
                    detail_b: 0,
                })?;
            let (stream, peer) = tokio::select! {
                _ = tokio::time::sleep(CAPTURE_TIMEOUT) => {
                    return Err(CaptureError {
                        error_code: 3,
                        detail_a: 0,
                        detail_b: 0,
                    });
                }
                _ = &mut cancel_receiver => return Ok(()),
                accepted = listener.accept() => accepted.map_err(|_| CaptureError {
                    error_code: 4,
                    detail_a: 0,
                    detail_b: 0,
                })?,
            };
            if peer.ip() != std::net::Ipv4Addr::LOCALHOST {
                return Err(CaptureError {
                    error_code: 5,
                    detail_a: 0,
                    detail_b: 0,
                });
            }
            let service = service_fn(move |request| {
                handle_responses_capture(
                    request,
                    sender.clone(),
                    server_request_count.clone(),
                    expected.clone(),
                    sensitive_needles.clone(),
                )
            });
            let mut builder = http1::Builder::new();
            builder.keep_alive(false);
            builder.max_headers(MAX_CAPTURE_HEADERS);
            builder.max_buf_size(MAX_CAPTURE_HEADER_BYTES);
            tokio::select! {
                _ = tokio::time::sleep(CAPTURE_TIMEOUT) => {
                    return Err(CaptureError {
                        error_code: 6,
                        detail_a: 0,
                        detail_b: 0,
                    });
                }
                _ = &mut cancel_receiver => return Ok(()),
                result = builder.serve_connection(TokioIo::new(stream), service) => {
                    result.map_err(|_| CaptureError {
                        error_code: 7,
                        detail_a: 0,
                        detail_b: 0,
                    })?;
                }
            }
            Ok(())
        })
    });
    OneShotResponsesCapture {
        base_url,
        receiver,
        request_count,
        cancel: Some(cancel),
        server: Some(server),
    }
}

fn cancel_one_shot_responses_capture(capture: OneShotResponsesCapture) {
    drop(capture);
}

fn finish_one_shot_responses_capture(
    mut capture: OneShotResponsesCapture,
) -> Result<CapturedResponsesRequest, CaptureError> {
    let received = capture
        .receiver
        .recv_timeout(CAPTURE_TIMEOUT)
        .map_err(|_| CaptureError {
            error_code: 8,
            detail_a: capture.request_count.load(Ordering::SeqCst),
            detail_b: 0,
        });
    if received.is_err() {
        if let Some(cancel) = capture.cancel.take() {
            let _ = cancel.send(());
        }
    }
    let server = capture
        .server
        .take()
        .expect("loopback capture server is available")
        .join()
        .map_err(|_| CaptureError {
            error_code: 9,
            detail_a: 0,
            detail_b: 0,
        })?;
    server?;
    let request_count = capture.request_count.load(Ordering::SeqCst);
    if request_count != 1 {
        return Err(CaptureError {
            error_code: 10,
            detail_a: request_count,
            detail_b: 1,
        });
    }
    received?
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

fn normalize_position_zero_marker(value: &str) -> Result<String, MarkerPrefixError> {
    let raw_marker_bytes = value.len();
    let prefix_bytes = value
        .bytes()
        .take_while(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
        .count();
    if raw_marker_bytes == 0
        || raw_marker_bytes > MAX_WRITE_PROMPT_BYTES
        || prefix_bytes == 0
        || prefix_bytes > 128
    {
        return Err(MarkerPrefixError {
            raw_marker_bytes,
            prefix_bytes,
        });
    }
    Ok(value[..prefix_bytes].to_string())
}

fn validate_exact_marker(value: &str) -> Result<String, MarkerPrefixError> {
    let raw_marker_bytes = value.len();
    let prefix_bytes = value
        .bytes()
        .take_while(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
        .count();
    if raw_marker_bytes == 0 || raw_marker_bytes > 128 || prefix_bytes != raw_marker_bytes {
        return Err(MarkerPrefixError {
            raw_marker_bytes,
            prefix_bytes,
        });
    }
    Ok(value.to_string())
}

fn is_single_exact_started_thread(started_thread_ids: &[String], expected: &str) -> bool {
    started_thread_ids.len() == 1 && started_thread_ids[0] == expected
}

fn compare_ordered_history(
    actual: &[HistoryMarker],
    expected: &[HistoryMarker],
) -> HistoryComparison {
    HistoryComparison {
        expected_items: expected.len(),
        actual_items: actual.len(),
        first_mismatch_index: actual
            .iter()
            .zip(expected)
            .position(|(actual, expected)| actual != expected)
            .unwrap_or_else(|| actual.len().min(expected.len())),
        role_mismatches: actual
            .iter()
            .zip(expected)
            .filter(|(actual, expected)| actual.role != expected.role)
            .count(),
        marker_mismatches: actual
            .iter()
            .zip(expected)
            .filter(|(actual, expected)| actual.marker != expected.marker)
            .count(),
    }
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
            "cli_version": "0.162.1",
            "source": "exec",
            "model_provider": PROVIDER_ID,
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
    let start_ordinal = records
        .first()
        .and_then(|record| record["payload"]["history_base"]["end_ordinal_exclusive"].as_u64())
        .unwrap_or(0);
    let mut bytes = Vec::new();
    for (offset, record) in records.iter_mut().enumerate() {
        record
            .as_object_mut()
            .expect("rollout record must be an object")
            .insert("ordinal".to_string(), json!(start_ordinal + offset as u64));
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

fn assert_rollout_provider(codex_home: &Path, provider_thread_id: &str) {
    let path = rollout_lookup(codex_home, provider_thread_id)
        .unwrap_or_else(|_| panic!("cannot inspect isolated Codex rollout provider"))
        .unwrap_or_else(|| panic!("isolated Codex rollout is unavailable"));
    let bytes = read_bounded_file(
        &path,
        MAX_CONVERSATION_FILE_BYTES,
        "isolated Codex rollout provider",
    );
    let first = bytes
        .split(|byte| *byte == b'\n')
        .find(|line| !line.is_empty())
        .expect("isolated Codex rollout is empty");
    let first: Value =
        serde_json::from_slice(first).expect("isolated Codex rollout metadata is malformed");
    assert_eq!(
        first["type"],
        json!("session_meta"),
        "isolated Codex rollout lacks leading session metadata"
    );
    assert_eq!(
        first["payload"]["model_provider"],
        json!(PROVIDER_ID),
        "isolated Codex rollout used an unexpected model provider"
    );
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
        exact_cutoff(&node.provider_thread_id, &bytes).end_ordinal_exclusive
    );
    let artifact = encode_standalone_codex_root(
        &resolved,
        &project_root.to_string_lossy().into_owned(),
        (canonicalize(project_root)
            .ok()
            .map(|path| path.to_string_lossy().into_owned()))
        .as_deref(),
        None,
        &mut mallard_infrastructure::credential_redaction::shared().begin_run(),
        &mallard_infrastructure::session_path_origins::PathOrigins::default(),
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
    let mut next_ordinal = None;
    for raw in bytes
        .split(|byte| *byte == b'\n')
        .filter(|raw| !raw.is_empty())
    {
        let record: Value = serde_json::from_slice(raw).expect("parse rollout cutoff record");
        let ordinal = record["ordinal"]
            .as_u64()
            .expect("rollout cutoff record lacks a global ordinal");
        if let Some(expected) = next_ordinal {
            assert_eq!(ordinal, expected, "rollout ordinals are not contiguous");
        }
        next_ordinal = Some(
            ordinal
                .checked_add(1)
                .expect("rollout ordinal cannot advance"),
        );
    }
    HistoryBaseFixture {
        thread_id: thread_id.to_string(),
        end_ordinal_exclusive: next_ordinal.expect("rollout cutoff has no records"),
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
    let start_ordinal = exact_cutoff(provider_thread_id, &before).end_ordinal_exclusive;
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
    encode_standalone_codex_root(
        &refreshed,
        &project_root.to_string_lossy().into_owned(),
        (canonicalize(project_root)
            .ok()
            .map(|path| path.to_string_lossy().into_owned()))
        .as_deref(),
        None,
        &mut mallard_infrastructure::credential_redaction::shared().begin_run(),
        &mallard_infrastructure::session_path_origins::PathOrigins::default(),
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
    assert!(
        !source_home.join("auth.json").exists(),
        "isolated Codex source home unexpectedly contains auth.json"
    );
    std::fs::create_dir_all(snapshot_home).expect("create headless Codex snapshot home");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(snapshot_home, std::fs::Permissions::from_mode(0o700))
            .expect("secure headless Codex snapshot home");
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
    assert!(
        !snapshot_home.join("auth.json").exists(),
        "headless Codex snapshot unexpectedly contains auth.json"
    );
}

fn timeline_history_from_standalone(
    root_bytes: &[u8],
    project_root: &Path,
    provider_thread_id: &str,
) -> Result<Vec<HistoryMarker>, TimelineHistoryError> {
    let failure = |error_code, entry_index, detail_a, detail_b| TimelineHistoryError {
        error_code,
        entry_index,
        detail_a,
        detail_b,
    };
    if root_bytes.is_empty() || root_bytes.len() as u64 > MAX_CONVERSATION_FILE_BYTES {
        return Err(failure(1, 0, root_bytes.len(), 0));
    }

    let temporary = tempfile::tempdir().map_err(|_| failure(2, 0, 0, 0))?;
    let rollout = temporary.path().join("standalone-rollout.jsonl");
    std::fs::write(&rollout, root_bytes).map_err(|_| failure(3, 0, 0, 0))?;

    let canonical_project_root =
        canonicalize(project_root).map_err(|_| failure(4, 0, 0, 0))?;
    let mut project_root_aliases = vec![project_root.to_path_buf(), canonical_project_root.clone()];
    project_root_aliases.sort();
    project_root_aliases.dedup();
    let registry = AgentProviderRegistry::production();
    let adapter = registry
        .require(&codex_key())
        .map_err(|_| failure(5, 0, 0, 0))?;
    let context = SessionTimelineContext {
        resource_id: "codex:conversation:live-history-oracle",
        provider_session_id: provider_thread_id,
        project_root: &canonical_project_root,
        project_root_aliases: &project_root_aliases,
        provider_home: temporary.path(),
        child: None,
    };
    let index = adapter
        .build_timeline_index(&rollout, &context)
        .map_err(|_| failure(6, 0, 0, 0))?;
    if index.truncated {
        return Err(failure(7, 0, index.events.len(), index.warnings.len()));
    }
    if index.counts.prompts.quality != SessionCountQuality::Exact
        || index.counts.responses.quality != SessionCountQuality::Exact
    {
        return Err(failure(8, 0, index.events.len(), index.warnings.len()));
    }
    let prompt_count = index
        .counts
        .prompts
        .value
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| failure(9, 0, 0, 0))?;
    let response_count = index
        .counts
        .responses
        .value
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| failure(10, 0, 0, 0))?;
    let display_count = prompt_count
        .checked_add(response_count)
        .ok_or_else(|| failure(11, 0, prompt_count, response_count))?;
    if display_count > MAX_SESSION_DETAIL_EVENTS {
        return Err(failure(12, 0, display_count, MAX_SESSION_DETAIL_EVENTS));
    }

    let included_event_kinds = BTreeSet::from([
        SessionTimelineEventKind::Prompt,
        SessionTimelineEventKind::Response,
    ]);
    let mut start_index = None;
    let mut page_count = 0usize;
    let mut event_ids = BTreeSet::new();
    let mut previous_ordinal = None;
    let mut history = Vec::with_capacity(display_count);
    loop {
        page_count = page_count.saturating_add(1);
        if page_count > MAX_SESSION_DETAIL_EVENTS {
            return Err(failure(13, history.len(), page_count, history.len()));
        }
        let page = adapter
            .read_timeline_page(
                &rollout,
                &index,
                &SessionTimelineQuery {
                    start_index,
                    limit: HISTORY_TIMELINE_PAGE_LIMIT,
                    included_event_kinds: included_event_kinds.clone(),
                    included_tool_categories: BTreeSet::new(),
                    turn_id: None,
                },
                &context,
            )
            .map_err(|_| failure(14, history.len(), page_count, 0))?;
        let page_event_count = page.events.len();
        if page_event_count > HISTORY_TIMELINE_PAGE_LIMIT {
            return Err(failure(
                15,
                history.len(),
                page_event_count,
                HISTORY_TIMELINE_PAGE_LIMIT,
            ));
        }
        for event in page.events {
            let entry_index = history.len();
            if history.len() >= MAX_SESSION_DETAIL_EVENTS {
                return Err(failure(
                    16,
                    entry_index,
                    history.len(),
                    MAX_SESSION_DETAIL_EVENTS,
                ));
            }
            if !event_ids.insert(event.event_id) {
                return Err(failure(17, entry_index, 0, 0));
            }
            if previous_ordinal.is_some_and(|previous| event.ordinal <= previous) {
                return Err(failure(
                    18,
                    entry_index,
                    usize::try_from(event.ordinal).unwrap_or(usize::MAX),
                    0,
                ));
            }
            previous_ordinal = Some(event.ordinal);
            if event.quality != SessionCountQuality::Exact {
                return Err(failure(19, entry_index, 0, 0));
            }
            let text = event.text.ok_or_else(|| failure(20, entry_index, 0, 0))?;
            let (role, marker) = match event.kind {
                SessionTimelineEventKind::Prompt => {
                    (HistoryRole::User, normalize_position_zero_marker(&text))
                }
                SessionTimelineEventKind::Response => {
                    (HistoryRole::Assistant, validate_exact_marker(&text))
                }
                _ => return Err(failure(21, entry_index, 0, 0)),
            };
            let marker = marker.map_err(|error| {
                failure(22, entry_index, error.raw_marker_bytes, error.prefix_bytes)
            })?;
            history.push(HistoryMarker { role, marker });
        }

        let Some(next_index) = page.next_index else {
            break;
        };
        let previous_start = start_index.unwrap_or(0);
        if next_index <= previous_start || next_index > index.events.len() {
            return Err(failure(23, history.len(), previous_start, next_index));
        }
        if page_event_count == 0 {
            return Err(failure(24, history.len(), previous_start, next_index));
        }
        start_index = Some(next_index);
    }

    let actual_prompts = history
        .iter()
        .filter(|entry| entry.role == HistoryRole::User)
        .count();
    let actual_responses = history
        .iter()
        .filter(|entry| entry.role == HistoryRole::Assistant)
        .count();
    if actual_prompts != prompt_count || actual_responses != response_count {
        return Err(failure(25, history.len(), actual_prompts, actual_responses));
    }
    Ok(history)
}

fn resolved_local_alias_history(
    codex_home: &Path,
    project_root: &Path,
    provider_thread_id: &str,
    root_tail_layout: Option<&mallard_domain::RootTailLayout>,
    replica_index: usize,
    history_index: usize,
) -> Vec<HistoryMarker> {
    let path = redacted(
        rollout_lookup(codex_home, provider_thread_id),
        "cannot locate exact local history alias",
    )
    .unwrap_or_else(|| panic!("exact local history alias is unavailable"));
    let lookup = |thread_id: &str| rollout_lookup(codex_home, thread_id);
    let resolved = redacted(
        resolve_codex_root(&path, &lookup),
        "production resolver rejected exact local history alias",
    );
    let artifact = redacted(
        encode_standalone_codex_root(
            &resolved,
            &project_root.to_string_lossy().into_owned(),
            (canonicalize(project_root)
                .ok()
                .map(|path| path.to_string_lossy().into_owned()))
            .as_deref(),
            root_tail_layout,
            &mut mallard_infrastructure::credential_redaction::shared().begin_run(),
            &mallard_infrastructure::session_path_origins::PathOrigins::default(),
        ),
        "production standalone builder rejected exact local history alias",
    );
    timeline_history_from_standalone(
        &artifact.root_bytes,
        project_root,
        &artifact.root_provider_thread_id,
    )
    .unwrap_or_else(|error| {
        panic!(
            "exact local alias timeline projection failed (replica_index={replica_index}, history_index={history_index}, error_code={}, entry_index={}, detail_a={}, detail_b={})",
            error.error_code, error.entry_index, error.detail_a, error.detail_b,
        )
    })
}

fn assert_resolved_local_alias_history(
    codex_home: &Path,
    project_root: &Path,
    provider_thread_id: &str,
    root_tail_layout: Option<&mallard_domain::RootTailLayout>,
    expected: &[HistoryMarker],
    replica_index: usize,
    history_index: usize,
) {
    let actual = resolved_local_alias_history(
        codex_home,
        project_root,
        provider_thread_id,
        root_tail_layout,
        replica_index,
        history_index,
    );
    let comparison = compare_ordered_history(&actual, expected);
    assert!(
        actual == expected,
        "resolved local alias history differs from its exact oracle (replica_index={replica_index}, history_index={history_index}, expected_items={}, actual_items={}, first_mismatch_index={}, role_mismatches={}, marker_mismatches={})",
        comparison.expected_items,
        comparison.actual_items,
        comparison.first_mismatch_index,
        comparison.role_mismatches,
        comparison.marker_mismatches,
    );
}

#[allow(clippy::too_many_arguments)]
fn run_headless_history_verification(
    codex_turn_count: &mut usize,
    sensitive_needles: &[Vec<u8>],
    source_codex_home: &Path,
    project_root: &Path,
    provider_thread_id: &str,
    expected: &[HistoryMarker],
    replica_index: usize,
    history_index: usize,
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
    assert_rollout_provider(&snapshot_home, provider_thread_id);
    let rollout_before = read_bounded_file(
        &snapshot_rollout,
        MAX_CONVERSATION_FILE_BYTES,
        "snapshot rollout before headless resume",
    );
    assert!(
        !expected.is_empty(),
        "headless history oracle must contain at least one display message"
    );
    assert!(
        HEADLESS_REOPEN_PROMPT.len() <= MAX_PROMPT_BYTES
            && !HEADLESS_REOPEN_PROMPT.chars().any(char::is_control),
        "headless verification prompt violates the argument bound"
    );
    let capture = start_one_shot_responses_capture(expected, sensitive_needles);
    let mut arguments = vec![
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
    ];
    arguments.extend(capture_provider_config_arguments(&capture.base_url));
    arguments.extend([
        OsString::from("--cd"),
        project_root.as_os_str().to_owned(),
        OsString::from("resume"),
        OsString::from("--ephemeral"),
        OsString::from(provider_thread_id),
        OsString::from(HEADLESS_REOPEN_PROMPT),
    ]);
    assert!(
        arguments.len() <= MAX_COMMAND_ARGUMENTS
            && arguments
                .iter()
                .all(|argument| argument.as_encoded_bytes().len() <= MAX_COMMAND_ARGUMENT_BYTES),
        "headless Codex command violates the argv safety bound"
    );
    for argument in &arguments {
        assert_no_sensitive_bytes(
            argument.as_encoded_bytes(),
            sensitive_needles,
            "headless Codex command argument",
        );
    }

    let mut command = Command::new(codex_binary());
    configure_codex_capture_child(&mut command);
    let child = command
        .args(arguments)
        .current_dir(project_root)
        .env("CODEX_HOME", &snapshot_home)
        .env("HOME", &snapshot_process_home)
        .env("USERPROFILE", &snapshot_process_home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(_) => {
            cancel_one_shot_responses_capture(capture);
            panic!("cannot start headless Codex history verification");
        }
    };
    *codex_turn_count = codex_turn_count.saturating_add(1);
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            cancel_one_shot_responses_capture(capture);
            panic!("cannot capture headless Codex stdout");
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            cancel_one_shot_responses_capture(capture);
            panic!("cannot capture headless Codex stderr");
        }
    };
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
    let mut wait_failed = false;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(_) => {
                wait_failed = true;
                let _ = child.kill();
                break;
            }
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let status = child.wait();
    let stdout = stdout_reader.join();
    let stderr = stderr_reader.join();
    let capture_result = if timed_out || wait_failed {
        cancel_one_shot_responses_capture(capture);
        None
    } else {
        Some(finish_one_shot_responses_capture(capture))
    };
    if timed_out {
        panic!("headless Codex history verification exceeded its timeout");
    }
    if wait_failed {
        panic!("cannot wait for headless Codex history verification");
    }
    let status = status.unwrap_or_else(|_| panic!("cannot finish headless Codex verification"));
    let stdout = stdout
        .unwrap_or_else(|_| panic!("headless stdout reader panicked"))
        .unwrap_or_else(|_| panic!("cannot collect headless Codex stdout"));
    let stderr = stderr
        .unwrap_or_else(|_| panic!("headless stderr reader panicked"))
        .unwrap_or_else(|_| panic!("cannot collect headless Codex stderr"));
    let captured = capture_result
        .expect("headless capture result is available")
        .unwrap_or_else(|error| {
            panic!(
                "loopback Responses capture failed (replica_index={replica_index}, history_index={history_index}, error_code={}, detail_a={}, detail_b={})",
                error.error_code, error.detail_a, error.detail_b,
            )
        });
    assert!(
        stdout.len() <= MAX_CODEX_OUTPUT_BYTES && stderr.len() <= MAX_CODEX_OUTPUT_BYTES,
        "headless Codex verification output exceeds its bound"
    );
    assert_no_sensitive_bytes(&stdout, sensitive_needles, "headless Codex stdout");
    assert_no_sensitive_bytes(&stderr, sensitive_needles, "headless Codex stderr");
    assert_codex_home_has_no_sensitive_state(&snapshot_home, sensitive_needles);
    assert_process_home_has_no_sensitive_state(&snapshot_process_home, sensitive_needles);
    assert!(
        captured.method == Method::POST
            && captured.path == "/v1/responses"
            && !captured.has_query
            && !captured.content_encoding_present
            && captured.content_type_is_json
            && !captured.authorization_present,
        "headless Codex did not use the exact unauthenticated loopback Responses boundary"
    );
    assert!(
        captured.history_items == expected.len()
            && captured.input_items > captured.history_items
            && captured.probe_input_index < captured.input_items,
        "validated loopback request report is internally inconsistent (replica_index={replica_index}, history_index={history_index}, expected_items={}, history_items={}, input_items={}, probe_input_index={})",
        expected.len(),
        captured.history_items,
        captured.input_items,
        captured.probe_input_index,
    );
    assert!(
        status.success(),
        "headless Codex verification failed without exposing captured content"
    );

    let stdout = redacted(
        String::from_utf8(stdout),
        "headless Codex JSON output was not UTF-8",
    );
    let mut started_thread_ids = Vec::new();
    let mut final_messages = Vec::new();
    let mut turn_started = false;
    for line in stdout.lines() {
        let event: Value = redacted(
            serde_json::from_str(line),
            "headless Codex emitted malformed JSONL",
        );
        if codex_output::is_startup_warning(&event, turn_started, MODEL) {
            continue;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("turn.started") => turn_started = true,
            Some("thread.started") => {
                started_thread_ids.push(
                    event
                        .get("thread_id")
                        .and_then(Value::as_str)
                        .expect("thread.started event lacks its thread identity")
                        .to_string(),
                );
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
                    Some(_) => panic!("headless verification used a forbidden completed item"),
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
                    "headless history verification started a forbidden item"
                );
            }
            _ => {}
        }
    }
    assert!(
        is_single_exact_started_thread(&started_thread_ids, provider_thread_id),
        "headless verification did not return exactly one matching thread identity"
    );
    assert_eq!(
        final_messages.len(),
        1,
        "headless verification did not return exactly one final response"
    );
    assert_eq!(
        final_messages[0], HEADLESS_REOPEN_ACK,
        "headless verification did not emit the fixed loopback acknowledgement"
    );

    // Codex 0.152 appends to an existing rollout despite --ephemeral. Always
    // use a fresh disposable snapshot, prove the exact target was resumed and
    // extended there, and leave authoritative isolated history untouched.
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
    codex_turn_count: &mut usize,
    sensitive_needles: &[Vec<u8>],
    repositories: [(&LocalStateRepository, &LocalProjectId, &Path, &Path); 2],
    histories: &[(CloudThreadId, Vec<HistoryMarker>)],
) {
    for (replica_index, (repository, project_id, codex_home, project_root)) in
        repositories.into_iter().enumerate()
    {
        let before = digest_inventory(codex_home);
        for (history_index, (cloud_thread_id, expected)) in histories.iter().enumerate() {
            // Each replica resolves the shared cloud identity to its own
            // machine-local registration.
            let logical_thread_id = local_thread_of_cloud(repository, project_id, cloud_thread_id);
            let binding = conversation_binding(repository, project_id, &logical_thread_id);
            let provider_id = binding
                .provider_thread_id
                .as_deref()
                .expect("logical history alias is not resumable");
            assert_resolved_local_alias_history(
                codex_home,
                project_root,
                provider_id,
                binding.root_tail_layout.as_ref(),
                expected,
                replica_index,
                history_index,
            );
            run_headless_history_verification(
                codex_turn_count,
                sensitive_needles,
                codex_home,
                project_root,
                provider_id,
                expected,
                replica_index,
                history_index,
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
    local_storage_config(
        StorageId::parse("storage-live-codex-e2e").expect("static storage id"),
        "Live Codex local storage",
        storage_root,
    )
}

fn codex_key() -> ProviderKey {
    ProviderKey::parse("codex").expect("static provider key")
}

fn finish_setup(
    repository: &LocalStateRepository,
    service: &ProjectSyncService<'_>,
    project_root: &Path,
    codex_home: &Path,
    storage: StorageConfig,
    repository_choice: DraftRepositoryChoice,
) -> ProjectDetail {
    let storage_id = storage.id.clone();
    let mut config = redacted(
        repository.load_config(),
        "cannot load storage configuration",
    );
    if !config.storages.iter().any(|stored| stored.id == storage_id) {
        config.storages.push(storage);
        redacted(
            repository.save_config(config),
            "cannot save storage configuration",
        );
    }
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
    draft.storage = Some(DraftStorageSelection { storage_id });
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
        "cannot plan reviewed Pull",
    );
    let result = redacted(
        service.apply_bundle_restore(&plan, approved_pull_actions(&plan)),
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
    let encoded = serde_json::to_string(path).unwrap();
    let path = encoded.trim_matches('"');
    assert!(
        byte_slice_contains(bytes, path.as_bytes()),
        "{label} does not contain an expected path"
    );
}

fn assert_bytes_exclude_path(bytes: &[u8], path: &Path, label: &str) {
    let path = path
        .to_str()
        .expect("live path assertion requires a UTF-8 path");
    let encoded = serde_json::to_string(path).unwrap();
    let path = encoded.trim_matches('"');
    assert!(
        !byte_slice_contains(bytes, path.as_bytes()),
        "{label} retained a forbidden path"
    );
}

fn try_file_change_keys(bytes: &[u8]) -> Result<BTreeSet<String>, &'static str> {
    let mut file_changes = Vec::new();
    for raw in bytes
        .split(|byte| *byte == b'\n')
        .filter(|raw| !raw.is_empty())
    {
        let record: Value = serde_json::from_slice(raw).map_err(|_| "malformed_rollout_jsonl")?;
        if record.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let payload = record.get("payload").ok_or("item_payload_missing")?;
        if payload.get("type").and_then(Value::as_str) != Some("item_completed") {
            continue;
        }
        let item = payload.get("item").ok_or("completed_item_missing")?;
        if item.get("type").and_then(Value::as_str) != Some("FileChange") {
            continue;
        }
        if item.get("status").and_then(Value::as_str) != Some("completed") {
            return Err("file_change_not_completed");
        }
        let changes = item
            .get("changes")
            .and_then(Value::as_object)
            .ok_or("file_change_paths_missing")?;
        file_changes.push(changes.keys().cloned().collect::<BTreeSet<_>>());
    }
    if file_changes.len() != 1 {
        return Err("file_change_count_not_one");
    }
    file_changes.pop().ok_or("file_change_count_not_one")
}

fn rollout_shape_synopsis(bytes: &[u8]) -> String {
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
        let outer = record.get("type").and_then(Value::as_str).unwrap_or("-");
        let payload = record.get("payload").unwrap_or(&record);
        let inner = payload.get("type").and_then(Value::as_str).unwrap_or("-");
        let name = payload.get("name").and_then(Value::as_str).unwrap_or("-");
        *shapes.entry(format!("{outer}/{inner}/{name}")).or_default() += 1;
    }
    shapes
        .into_iter()
        .map(|(shape, count)| format!("{shape}={count}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn file_change_keys(bytes: &[u8]) -> BTreeSet<String> {
    try_file_change_keys(bytes).unwrap_or_else(|reason| {
        panic!("path-bearing rollout file-change evidence failed: {reason}")
    })
}

fn local_file_change_key_candidates(
    project_root: &Path,
    outside_root: &Path,
) -> Vec<BTreeSet<String>> {
    let canonical_project =
        canonicalize(project_root).expect("canonicalize patch-key project root");
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

fn assert_local_file_change_keys(bytes: &[u8], project_root: &Path, outside_root: &Path) {
    let actual = file_change_keys(bytes);
    assert!(
        local_file_change_key_candidates(project_root, outside_root)
            .iter()
            .any(|expected| expected == &actual),
        "materialized rollout file-change keys do not exactly match the local and outside fixtures"
    );
}

fn assert_portable_file_change_keys(bytes: &[u8], outside_root: &Path) {
    let actual = file_change_keys(bytes);
    let candidates = ["b", "c"].map(|style| {
        IN_PROJECT_FIXTURES
            .iter()
            .map(|(relative, _)| format!("/__mallard_root__/{style};{}{relative}", std::path::MAIN_SEPARATOR))
            .chain(
                canonical_fixture_paths(outside_root, &OUTSIDE_PROJECT_FIXTURES)
                    .into_iter()
                    .map(|path| path.to_string_lossy().into_owned()),
            )
            .collect::<BTreeSet<_>>()
    });
    assert!(
        candidates.contains(&actual),
        "portable rollout file-change keys did not tokenize only in-project fixtures"
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
    assert_local_file_change_keys(&bytes, expected_project_root, outside_root);
    for (relative, _) in IN_PROJECT_FIXTURES {
        let spellings = fixture_path_spellings(expected_project_root, relative);
        assert!(
            spellings
                .iter()
                .any(|path| {
                    let encoded = serde_json::to_string(path).unwrap();
                    byte_slice_contains(&bytes, encoded.trim_matches('"').as_bytes())
                }),
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
    cloud_thread_id: &CloudThreadId,
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
        .get(cloud_thread_id)
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
    cloud_thread_id: &CloudThreadId,
    source_project_root: &Path,
    target_project_root: &Path,
    outside_root: &Path,
) {
    let bytes = portable_conversation_rollout(
        service,
        storage_id,
        bundle_id,
        storage_root,
        cloud_thread_id,
    );
    assert_portable_file_change_keys(&bytes, outside_root);
    for (relative, _) in IN_PROJECT_FIXTURES {
        assert!(
            ["b", "c"].iter().any(|style| {
                let token = format!("/__mallard_root__/{style};{}{relative}", std::path::MAIN_SEPARATOR);
                let encoded = serde_json::to_string(&token).unwrap();
                byte_slice_contains(&bytes, encoded.trim_matches('"').as_bytes())
            }),
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
        canonicalize(project_root).expect("canonicalize Session Details project root");
    let canonical_home =
        canonicalize(codex_home).expect("canonicalize Session Details provider home");
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

/// The cloud identity a local thread is mapped to — the machine-global half
/// of the binding-row pairing, and the only cross-replica rendezvous value.
fn cloud_thread_id_of(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    logical_thread_id: &ThreadId,
) -> CloudThreadId {
    conversation_binding(repository, project_id, logical_thread_id)
        .cloud_thread_id
        .expect("thread has no cloud mapping")
}

/// The machine-local thread registered for one cloud identity.
fn local_thread_of_cloud(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    cloud_thread_id: &CloudThreadId,
) -> ThreadId {
    redacted(
        repository.load_conversation_bindings(),
        "cannot load conversation bindings",
    )
    .bindings
    .into_iter()
    .find(|binding| {
        binding.local_project_id == *project_id
            && binding.cloud_thread_id.as_ref() == Some(cloud_thread_id)
    })
    .map(|binding| binding.thread_id)
    .expect("cloud thread has no local registration")
}

/// Plans are pure intent: the divergent offer names the local thread and
/// mints nothing — every keep-both identity exists only after apply.
fn assert_keep_both_offered(plan: &RestorePlan, logical_thread_id: &ThreadId) {
    plan.conversation_review
        .iter()
        .find(|row| {
            row.thread_id.as_ref() == Some(logical_thread_id)
                && row.action == ConversationReviewAction::KeepBoth
        })
        .expect("divergent Pull does not offer Keep Both");
}

/// The identities a keep-both apply minted, read back from the receipt and
/// the committed binding rows.
struct KeepBothOutcome {
    preserved_thread_id: ThreadId,
    remote_provider_thread_id: String,
}

fn assert_keep_both_result(
    repository: &LocalStateRepository,
    project_id: &LocalProjectId,
    logical_thread_id: &ThreadId,
    cloud_thread_id: &CloudThreadId,
    old_provider_thread_id: &str,
    result: &RestoreResult,
) -> KeepBothOutcome {
    assert_eq!(result.preserved_threads.len(), 1);
    let preserved = &result.preserved_threads[0];
    assert_eq!(preserved.original_thread_id, *logical_thread_id);
    assert!(preserved.preserved_display_title.contains("Preserved "));
    let preserved_thread_id = preserved.preserved_thread_id.clone();
    // The tracking row keeps its local id and cloud mapping and receives the
    // apply-minted remote alias.
    let tracking = conversation_binding(repository, project_id, logical_thread_id);
    assert_eq!(
        tracking.cloud_thread_id.as_ref(),
        Some(cloud_thread_id),
        "Keep Both dropped the tracking row's cloud mapping"
    );
    let remote_provider_thread_id = tracking
        .provider_thread_id
        .clone()
        .expect("Keep Both minted the remote branch's provider id at apply");
    assert_ne!(
        remote_provider_thread_id, old_provider_thread_id,
        "Keep Both reused the divergent provider thread for the cloud side"
    );
    assert_eq!(
        provider_thread_id(repository, project_id, &preserved_thread_id),
        old_provider_thread_id,
        "Keep Both did not retain the local provider thread on the preserved logical ID"
    );
    let preserved_row = conversation_binding(repository, project_id, &preserved_thread_id);
    assert_eq!(
        preserved_row.cloud_thread_id, None,
        "the preserved branch kept a cloud mapping"
    );
    let provenance = preserved_row
        .preserved
        .expect("Keep Both did not persist preservation provenance");
    // Provenance names the divergent thread's cloud identity (portable space).
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
    let comparison = redacted(
        service.compare_threads(project_id, storage_id),
        "cannot refresh the divergent thread comparison",
    );
    assert!(
        comparison.entries.iter().any(|entry| {
            entry.cloud_thread_id.as_ref() == Some(cloud_thread_id)
                && entry.state == ThreadSyncState::Diverged
        }),
        "{replica}: the first comparison must detect the completed native update"
    );

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
            entry.cloud_thread_id.as_ref() == Some(cloud_thread_id)
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
    expected: &[(&CloudThreadId, ThreadSyncState)],
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
    review: &mallard_application::project_sync::service::PushReviewContext,
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
    assert_eq!(records[0]["payload"]["model_provider"], json!(PROVIDER_ID));
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
    let artifact = encode_standalone_codex_root(
        &resolved,
        &project_root.to_string_lossy().into_owned(),
        (canonicalize(project_root)
            .ok()
            .map(|path| path.to_string_lossy().into_owned()))
        .as_deref(),
        None,
        &mut mallard_infrastructure::credential_redaction::shared().begin_run(),
        &mallard_infrastructure::session_path_origins::PathOrigins::default(),
    )
    .expect("production standalone builder rejected pulled rollout");
    let actual = timeline_history_from_standalone(
        &artifact.root_bytes,
        project_root,
        &artifact.root_provider_thread_id,
    )
    .unwrap_or_else(|error| {
        panic!(
            "pulled standalone timeline projection failed (error_code={}, entry_index={}, detail_a={}, detail_b={})",
            error.error_code, error.entry_index, error.detail_a, error.detail_b,
        )
    });
    let comparison = compare_ordered_history(&actual, expected);
    assert!(
        actual == expected,
        "pulled standalone rollout history is incorrect (expected_items={}, actual_items={}, first_mismatch_index={}, role_mismatches={}, marker_mismatches={})",
        comparison.expected_items,
        comparison.actual_items,
        comparison.first_mismatch_index,
        comparison.role_mismatches,
        comparison.marker_mismatches,
    );
}

fn byte_slice_contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn credential_needles(api_key: &[u8]) -> Vec<Vec<u8>> {
    let digest = Sha256::digest(api_key);
    let mut needles = vec![api_key.to_vec(), digest.to_vec()];
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

fn is_generated_plugin_cache(relative: &Path) -> bool {
    let components = relative.components().collect::<Vec<_>>();
    let normal = |index: usize, expected: &str| {
        matches!(
            components.get(index),
            Some(std::path::Component::Normal(value)) if *value == std::ffi::OsStr::new(expected)
        )
    };
    (normal(0, "plugins") && normal(1, "cache")) || (normal(0, ".tmp") && normal(1, "marketplaces"))
}

fn is_generated_arg0_wrapper(relative: &Path) -> bool {
    let components = relative.components().collect::<Vec<_>>();
    if components.len() != 4 {
        return false;
    }
    let normal = |index: usize| match components.get(index) {
        Some(std::path::Component::Normal(value)) => value.to_str(),
        _ => None,
    };
    let directory_is_valid = normal(2).is_some_and(|value| {
        value.starts_with("codex-arg0")
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    });
    normal(0) == Some("tmp")
        && normal(1) == Some("arg0")
        && directory_is_valid
        && matches!(
            normal(3),
            Some("apply_patch" | "applypatch" | "codex-execve-wrapper")
        )
}

fn assert_generated_codex_symlink_is_safe(
    canonical_root: &Path,
    path: &Path,
    relative: &Path,
    sensitive: &[Vec<u8>],
) {
    let is_plugin_cache = is_generated_plugin_cache(relative);
    let is_arg0_wrapper = is_generated_arg0_wrapper(relative);
    assert!(
        is_plugin_cache || is_arg0_wrapper,
        "isolated provider state contains an unexpected symlink"
    );
    let target = std::fs::read_link(path).expect("inspect generated Codex symlink");
    assert_no_sensitive_bytes(
        target.as_os_str().as_encoded_bytes(),
        sensitive,
        "generated Codex symlink target",
    );
    let canonical_target = canonicalize(path).expect("resolve generated Codex symlink");
    if is_plugin_cache {
        assert!(
            canonical_target.starts_with(canonical_root),
            "generated plugin-cache symlink escapes the isolated provider home"
        );
        let target_relative = canonical_target
            .strip_prefix(canonical_root)
            .expect("generated plugin-cache target remains below its home");
        assert!(
            is_generated_plugin_cache(target_relative),
            "generated plugin-cache symlink targets another provider-state namespace"
        );
        assert_no_sensitive_bytes(
            target_relative.as_os_str().as_encoded_bytes(),
            sensitive,
            "generated plugin-cache resolved target",
        );
    } else {
        let expected_binary = canonicalize(codex_binary())
            .expect("canonicalize the installed Codex binary for wrapper validation");
        assert!(
            canonical_target == expected_binary,
            "generated Codex wrapper targets an unexpected executable"
        );
    }
}

fn assert_sensitive_state_tree(
    root: &Path,
    sensitive: &[Vec<u8>],
    allow_generated_cache_symlinks: bool,
) {
    let metadata = std::fs::symlink_metadata(root)
        .expect("inspect isolated provider state during privacy scan");
    assert!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "isolated provider state must be a no-follow directory"
    );
    let canonical_root =
        canonicalize(root).expect("canonicalize isolated provider-state root");
    let mut entries = 0_usize;
    let mut inspected_bytes = 0_u64;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = redacted(entry, "cannot inspect isolated Codex provider state");
        entries = entries.saturating_add(1);
        assert!(
            entries <= MAX_CODEX_HOME_PRIVACY_ENTRIES,
            "isolated Codex provider state exceeds its entry bound"
        );
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("Codex provider state remains below its home");
        assert_no_sensitive_bytes(
            relative.as_os_str().as_encoded_bytes(),
            sensitive,
            "isolated Codex provider-state path",
        );
        assert!(
            entry.file_name() != std::ffi::OsStr::new("auth.json"),
            "isolated provider state unexpectedly contains auth.json"
        );
        if entry.file_type().is_symlink() {
            assert!(
                allow_generated_cache_symlinks,
                "isolated provider state contains a symlink"
            );
            assert_generated_codex_symlink_is_safe(
                &canonical_root,
                entry.path(),
                relative,
                sensitive,
            );
            continue;
        }
        if entry.file_type().is_dir() {
            continue;
        }
        assert!(
            entry.file_type().is_file(),
            "isolated Codex provider state contains a special file"
        );
        let file_metadata =
            std::fs::symlink_metadata(entry.path()).expect("inspect Codex provider-state file");
        inspected_bytes = inspected_bytes.saturating_add(file_metadata.len());
        assert!(
            inspected_bytes <= MAX_CODEX_HOME_PRIVACY_BYTES,
            "isolated Codex provider state exceeds its byte bound"
        );
        let bytes = read_bounded_file(
            entry.path(),
            MAX_STORAGE_OBJECT_BYTES,
            "isolated Codex provider-state file",
        );
        assert_no_sensitive_bytes(&bytes, sensitive, "isolated Codex provider state");
    }
}

fn assert_codex_home_has_no_sensitive_state(codex_home: &Path, sensitive: &[Vec<u8>]) {
    assert!(
        !codex_home.join("auth.json").exists(),
        "isolated Codex home unexpectedly contains auth.json"
    );
    assert_sensitive_state_tree(codex_home, sensitive, true);
}

fn assert_process_home_has_no_sensitive_state(process_home: &Path, sensitive: &[Vec<u8>]) {
    assert_sensitive_state_tree(process_home, sensitive, false);
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
    let adapter = AgentProviderRegistry::production()
        .require(&codex_key())
        .expect("Codex session adapter is unavailable");
    let context = SessionTimelineContext {
        resource_id: "codex:conversation:offline-path-fixture",
        provider_session_id: "offline-path-fixture",
        project_root,
        project_root_aliases: &[],
        provider_home: rollout.parent().unwrap(),
        child: None,
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
            &context,
        )
        .expect("current Codex path fixture timeline did not load");
    assert_eq!(page.events.len(), 1);
    let detail = adapter
        .read_event_detail(rollout, &index, &tool_entry.event_id, &context)
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
fn live_provider_document_is_strict_and_builds_secret_free_codex_overrides() {
    let fake_key = "sk-fake-mallard-live-provider-key";
    let document = serde_json::to_vec(&json!({
        "_type": "newapi_channel_conn",
        "key": fake_key,
        "url": "https://provider.example.test",
    }))
    .expect("serialize fake provider document");
    let provider =
        parse_live_provider_connection(&document).expect("parse valid fake provider document");
    assert_eq!(provider.base_url, "https://provider.example.test/v1");
    let arguments = provider.config_arguments();
    assert!(arguments.len() <= MAX_COMMAND_ARGUMENTS);
    let encoded = arguments
        .iter()
        .map(|value| value.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(encoded.contains("model_provider=\"mallard_live\""));
    assert!(encoded.contains("model_reasoning_effort=\"low\""));
    assert!(encoded.contains("features.plugins=false"));
    assert!(encoded.contains("base_url=\"https://provider.example.test/v1\""));
    assert!(encoded.contains("env_key=\"MALLARD_CODEX_PROVIDER_API_KEY\""));
    assert!(encoded.contains("wire_api=\"responses\""));
    assert!(encoded.contains("requires_openai_auth=false"));
    assert!(!encoded.contains(fake_key));

    let metadata = session_meta_record(P0, Path::new("/tmp/project"), None);
    assert_eq!(metadata["payload"]["model_provider"], json!(PROVIDER_ID));
}

#[test]
fn live_provider_document_rejects_legacy_or_unsafe_shapes() {
    assert_eq!(
        normalize_provider_url("https://provider.example.test/v1"),
        Ok("https://provider.example.test/v1".to_string())
    );
    for url in [
        "http://provider.example.test",
        "https://user@provider.example.test",
        "https://provider.example.test/v2",
        "https://provider.example.test/v1/",
        "https://provider.example.test/v1?key=value",
        "https://provider.example.test/#fragment",
    ] {
        assert!(normalize_provider_url(url).is_err(), "accepted unsafe URL");
    }

    let invalid_documents = [
        json!({
            "_type": "legacy",
            "key": "sk-fake-mallard-live-provider-key",
            "url": "https://provider.example.test",
        }),
        json!({
            "_type": "newapi_channel_conn",
            "key": "sk-fake-mallard-live-provider-key",
            "url": "https://provider.example.test",
            "unknown": true,
        }),
        json!({
            "_type": "newapi_channel_conn",
            "key": "unsafe\nkey",
            "url": "https://provider.example.test",
        }),
        json!({
            "_type": "newapi_channel_conn",
            "key": "",
            "url": "https://provider.example.test",
        }),
    ];
    for document in invalid_documents {
        let bytes = serde_json::to_vec(&document).expect("serialize invalid provider document");
        assert!(
            parse_live_provider_connection(&bytes).is_err(),
            "accepted invalid provider document"
        );
    }
}

#[cfg(unix)]
#[test]
fn provider_state_scan_covers_cache_bodies_and_validates_generated_symlinks() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("create provider-state privacy fixture");
    let codex_home = root.path().join("myconf");
    create_secure_codex_home(&codex_home);
    let version_root = codex_home.join("plugins/cache/openai-bundled/chrome/fixture-version");
    std::fs::create_dir_all(&version_root).expect("create generated cache fixture");
    let descriptor = version_root.join("descriptor.txt");
    std::fs::write(&descriptor, b"generated cache fixture\n")
        .expect("write generated cache fixture");
    symlink(
        &version_root,
        codex_home.join("plugins/cache/openai-bundled/chrome/latest"),
    )
    .expect("create absolute generated cache symlink fixture");

    let sensitive = vec![b"FAKE_LIVE_CREDENTIAL_SENTINEL".to_vec()];
    assert_codex_home_has_no_sensitive_state(&codex_home, &sensitive);

    std::fs::write(&descriptor, &sensitive[0]).expect("write cache leak fixture");
    assert!(
        std::panic::catch_unwind(|| {
            assert_codex_home_has_no_sensitive_state(&codex_home, &sensitive)
        })
        .is_err(),
        "provider-state scan skipped a generated cache file body"
    );

    let process_home = root.path().join("process-home");
    std::fs::create_dir(&process_home).expect("create process-home privacy fixture");
    std::fs::write(process_home.join("auth.json"), b"fixture")
        .expect("write forbidden authentication fixture");
    assert!(
        std::panic::catch_unwind(|| {
            assert_process_home_has_no_sensitive_state(&process_home, &sensitive)
        })
        .is_err(),
        "process-home scan accepted an authentication file"
    );
}

#[test]
fn revision_one_prompt_and_diagnostics_preserve_only_safe_evidence() {
    let selected_project = Path::new(if cfg!(windows) {
        "C:/var/folders/mallard-live/project1"
    } else {
        "/var/folders/mallard-live/project1"
    });
    let canonical_external = Path::new(if cfg!(windows) {
        "C:/private/var/folders/mallard-live/project1_external"
    } else {
        "/private/var/folders/mallard-live/project1_external"
    });
    let prompt = revision_one_file_prompt(selected_project, canonical_external);
    let patch = revision_one_patch_body(canonical_external);
    let command = revision_one_patch_command(canonical_external);
    let exact_arguments = serde_json::to_string(&json!({
        "cmd": command,
        "workdir": selected_project.to_str().expect("fixture project path is UTF-8"),
    }))
    .expect("serialize expected exec arguments");
    assert_eq!(prompt.matches("exec_command").count(), 1);
    assert_eq!(prompt.matches(&exact_arguments).count(), 1);
    assert!(!prompt.contains("tools.apply_patch"));
    assert!(command.starts_with("apply_patch <<'MALLARD_PATCH'\n"));
    assert!(command.ends_with("*** End Patch\nMALLARD_PATCH\n"));
    assert_eq!(command.matches("apply_patch").count(), 1);
    assert_eq!(patch.matches("*** Begin Patch").count(), 1);
    assert_eq!(patch.matches("*** End Patch").count(), 1);
    assert_eq!(patch.matches("*** Add File:").count(), 4);
    assert!(patch.contains("*** Add File: ./generated/session-paths/inside-alpha.txt\n"));
    assert!(patch.contains("*** Add File: ./generated/session-paths/nested/inside-beta.json\n"));
    assert!(patch.contains(&format!(
        "*** Add File: {}/outside-alpha.txt\n", canonical_external.display()
    )));
    assert!(patch.contains(&format!(
        "*** Add File: {}/nested/outside-beta.json\n", canonical_external.display()
    )));
    assert!(patch.contains("+MALLARD_IN_PROJECT_ALPHA\n"));
    assert!(patch.contains("+{\"fixture\":\"MALLARD_IN_PROJECT_BETA\",\"version\":1}\n"));
    assert!(patch.contains("+MALLARD_OUTSIDE_ALPHA\n"));
    assert!(patch.contains("+{\"fixture\":\"MALLARD_OUTSIDE_BETA\",\"version\":1}\n"));
    assert!(!patch.contains("/var/folders/mallard-live/project1/"));
    assert!(prompt.contains(&serde_json::to_string(selected_project).unwrap()));
    assert!(!prompt.contains("/private/var/folders/mallard-live/project1/"));
    for (relative, _) in OUTSIDE_PROJECT_FIXTURES {
        let path = format!("{}/{relative}", canonical_external.display());
        let encoded = serde_json::to_string(&path).unwrap();
        assert!(prompt.contains(encoded.trim_matches('"')));
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
fn loopback_responses_capture_validates_complete_ordered_history() {
    let expected = marker_turn("LIVE_USER_R1", "MALLARD_BASE")
        .into_iter()
        .chain(marker_turn("GRAPH_USER_LEFT_PARENT", "GRAPH_LEFT_PARENT"))
        .collect::<Vec<_>>();
    let request = json!({
        "model": MODEL,
        "stream": true,
        "reasoning": {"effort": "low", "summary": "auto"},
        "input": [
            {
                "type": "message",
                "role": "developer",
                "content": [{"type": "input_text", "text": "LIVE_USER_IGNORED"}],
            },
            {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "<environment_context>ignored</environment_context>"}],
            },
            {"type": "function_call", "name": "ignored", "arguments": "{}"},
            {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "LIVE_USER_R1 with original instructions"}],
            },
            {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "MALLARD_BASE"}],
            },
            {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "GRAPH_USER_LEFT_PARENT with suffix"}],
            },
            {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "GRAPH_LEFT_PARENT"}],
            },
            {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": HEADLESS_REOPEN_PROMPT}],
            },
        ],
    });
    assert_eq!(
        validate_captured_responses_body(&request, &expected),
        Ok((8, 4, 7))
    );

    let mut extra_history = request.clone();
    extra_history["input"]
        .as_array_mut()
        .expect("capture fixture input")
        .insert(
            7,
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "GRAPH_EXTRA"}],
            }),
        );
    assert_eq!(
        validate_captured_responses_body(&extra_history, &expected)
            .expect_err("extra historical marker was accepted")
            .error_code,
        41
    );

    let mut assistant_suffix = request.clone();
    assistant_suffix["input"][4]["content"][0]["text"] = json!("MALLARD_BASE trailing");
    assert_eq!(
        validate_captured_responses_body(&assistant_suffix, &expected)
            .expect_err("assistant marker suffix was accepted")
            .error_code,
        39
    );

    let mut wrong_span = request.clone();
    wrong_span["input"][5]["content"][0]["type"] = json!("output_text");
    assert_eq!(
        validate_captured_responses_body(&wrong_span, &expected)
            .expect_err("user marker in an output span was accepted")
            .error_code,
        37
    );

    let mut probe_before_history = request.clone();
    let probe = probe_before_history["input"]
        .as_array_mut()
        .expect("capture fixture input")
        .pop()
        .expect("capture fixture probe");
    probe_before_history["input"]
        .as_array_mut()
        .expect("capture fixture input")
        .insert(3, probe);
    assert_eq!(
        validate_captured_responses_body(&probe_before_history, &expected)
            .expect_err("historical marker after the probe was accepted")
            .error_code,
        36
    );

    let requested = "019f7800-0000-7000-8000-000000000005";
    assert!(is_single_exact_started_thread(
        &[requested.to_string()],
        requested
    ));
    assert!(!is_single_exact_started_thread(&[], requested));
    assert!(!is_single_exact_started_thread(
        &["unexpected".to_string(), requested.to_string()],
        requested,
    ));
}

#[test]
fn production_timeline_history_projection_is_exact_and_paginated() {
    let root = tempfile::tempdir().expect("create timeline history fixture root");
    let project_root = root.path().join("project");
    std::fs::create_dir_all(&project_root).expect("create timeline history fixture project");
    let environment_context = format!(
        "<environment_context>{}</environment_context>",
        "x".repeat(1_536)
    );
    let mut fixture_records = vec![
        session_meta_record(P0, &project_root, None),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": fixture_uuid(0x500),
                "role": "user",
                "content": [{"type": "input_text", "text": environment_context}],
            },
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": fixture_uuid(0x501),
                "role": "developer",
                "content": [{"type": "input_text", "text": "DEVELOPER_NON_CHAT"}],
            },
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": fixture_uuid(0x502),
                "role": "system",
                "content": [{"type": "input_text", "text": "SYSTEM_NON_CHAT"}],
            },
        }),
        json!({
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "thread_id": P0,
                "turn_id": fixture_uuid(0x503),
                "item": {
                    "type": "UserMessage",
                    "id": fixture_uuid(0x504),
                    "content": [{"type": "text", "text": environment_context}],
                },
            },
        }),
    ];
    fixture_records.extend(turn_records(
        P0,
        &fixture_uuid(0x505),
        &fixture_uuid(0x506),
        &fixture_uuid(0x507),
        None,
        "GRAPH_USER_BASE_1 with a live-like suffix",
        "GRAPH_BASE_1",
    ));
    fixture_records.extend([
        json!({
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "thread_id": P0,
                "turn_id": fixture_uuid(0x505),
                "item": {
                    "type": "AgentMessage",
                    "id": fixture_uuid(0x508),
                    "phase": "commentary",
                    "content": [{"type": "text", "text": "COMMENTARY_NOT_CHAT_HISTORY"}],
                },
            },
        }),
        json!({
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "thread_id": P0,
                "turn_id": fixture_uuid(0x505),
                "item": {
                    "type": "CommandExecution",
                    "id": fixture_uuid(0x509),
                    "command": ["true"],
                    "status": "completed",
                    "stdout": "",
                    "stderr": "",
                },
            },
        }),
    ]);
    fixture_records.extend(turn_records(
        P0,
        &fixture_uuid(0x50a),
        &fixture_uuid(0x50b),
        &fixture_uuid(0x50c),
        Some(&fixture_uuid(0x507)),
        "GRAPH_USER_LEFT_PARENT continue",
        "GRAPH_LEFT_PARENT",
    ));
    fixture_records.extend(turn_records(
        P0,
        &fixture_uuid(0x50d),
        &fixture_uuid(0x50e),
        &fixture_uuid(0x50f),
        Some(&fixture_uuid(0x50c)),
        "GRAPH_USER_A_LOCAL_ONLY and another suffix",
        "GRAPH_A_LOCAL_ONLY",
    ));
    let expected = marker_turn("GRAPH_USER_BASE_1", "GRAPH_BASE_1")
        .into_iter()
        .chain(marker_turn("GRAPH_USER_LEFT_PARENT", "GRAPH_LEFT_PARENT"))
        .chain(marker_turn("GRAPH_USER_A_LOCAL_ONLY", "GRAPH_A_LOCAL_ONLY"))
        .collect::<Vec<_>>();
    let fixture_bytes = encode_rollout_records(fixture_records.clone());
    let actual = timeline_history_from_standalone(&fixture_bytes, &project_root, P0)
        .expect("project production timeline history fixture");
    assert_eq!(actual, expected);
    assert_eq!(
        compare_ordered_history(&actual, &expected),
        HistoryComparison {
            expected_items: 6,
            actual_items: 6,
            first_mismatch_index: 6,
            role_mismatches: 0,
            marker_mismatches: 0,
        }
    );

    let mut extra_chat_records = fixture_records.clone();
    extra_chat_records.push(json!({
        "type": "event_msg",
        "payload": {
            "type": "item_completed",
            "thread_id": P0,
            "turn_id": fixture_uuid(0x510),
            "item": {
                "type": "UserMessage",
                "id": fixture_uuid(0x511),
                "content": [{"type": "text", "text": "GRAPH_USER_EXTRA remains visible"}],
            },
        },
    }));
    let extra_chat_bytes = encode_rollout_records(extra_chat_records);
    let extra_chat = timeline_history_from_standalone(&extra_chat_bytes, &project_root, P0)
        .expect("project extra production timeline user message");
    assert_eq!(
        compare_ordered_history(&extra_chat, &expected),
        HistoryComparison {
            expected_items: 6,
            actual_items: 7,
            first_mismatch_index: 6,
            role_mismatches: 0,
            marker_mismatches: 0,
        }
    );
    assert_ne!(extra_chat, expected);

    let mut assistant_suffix = vec![session_meta_record(P0, &project_root, None)];
    assistant_suffix.extend(turn_records(
        P0,
        &fixture_uuid(0x512),
        &fixture_uuid(0x513),
        &fixture_uuid(0x514),
        None,
        "GRAPH_USER_BASE_1 with a live-like suffix",
        "GRAPH_BASE_1.",
    ));
    let assistant_suffix_bytes = encode_rollout_records(assistant_suffix);
    let error = timeline_history_from_standalone(&assistant_suffix_bytes, &project_root, P0)
        .expect_err("stored assistant marker suffix was accepted");
    assert_eq!(error.error_code, 22);
    assert_eq!(error.entry_index, 1);
    assert_eq!(error.detail_a, "GRAPH_BASE_1.".len());
    assert_eq!(error.detail_b, "GRAPH_BASE_1".len());
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
    let project1 = canonicalize(project1).expect("canonicalize source fixture project");
    let project1_copy =
        canonicalize(project1_copy).expect("canonicalize target fixture project");
    let project1_external =
        canonicalize(project1_external).expect("canonicalize outside fixture root");
    let source_thread_id = fixture_uuid(0x401);
    let target_thread_id = fixture_uuid(0x402);
    let inside_paths = canonical_fixture_paths(&project1, &IN_PROJECT_FIXTURES);
    let outside_paths = canonical_fixture_paths(&project1_external, &OUTSIDE_PROJECT_FIXTURES);
    let changes = json!({
        (inside_paths[0].to_string_lossy().into_owned()): {"type": "add"},
        (inside_paths[1].to_string_lossy().into_owned()): {"type": "add"},
        (outside_paths[0].to_string_lossy().into_owned()): {"type": "add"},
        (outside_paths[1].to_string_lossy().into_owned()): {"type": "add"},
    });
    let user_message_id = fixture_uuid(0x404);
    let assistant_message_id = fixture_uuid(0x405);
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
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "thread_id": source_thread_id,
                "turn_id": "turn-path-fixture",
                "item": {
                    "type": "FileChange",
                    "id": "file-change-path-fixture",
                    "status": "completed",
                    "changes": changes.clone(),
                },
            },
        }),
        json!({
            "timestamp": "2026-08-18T00:00:03Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": assistant_message_id,
                "parent_id": user_message_id,
                "role": "assistant",
                "content": [{"type": "output_text", "text": "MALLARD_BASE"}],
                "phase": "final_answer",
            },
        }),
    ];
    let mut records = vec![session_meta_record(&source_thread_id, &project1, None)];
    records.extend(turn);
    let source_bytes = encode_rollout_records(records);
    assert_local_file_change_keys(&source_bytes, &project1, &project1_external);
    let source_rollout = root.path().join("source.jsonl");
    std::fs::write(&source_rollout, &source_bytes).expect("write source path fixture rollout");
    assert_offline_path_timeline(
        &source_rollout,
        &project1,
        &project1,
        &project1_external,
        "file-change-path-fixture",
    );

    let no_bases = |_: &str| Ok(None);
    let source = resolve_codex_root(&source_rollout, &no_bases)
        .expect("resolve source current Codex path fixture");
    let wire = encode_standalone_codex_root(
        &source,
        &project1.to_string_lossy().into_owned(),
        (Some(project1.to_string_lossy().into_owned())).as_deref(),
        None,
        &mut mallard_infrastructure::credential_redaction::shared().begin_run(),
        &mallard_infrastructure::session_path_origins::PathOrigins::default(),
    )
    .expect("encode current Codex path fixture");
    assert_portable_file_change_keys(&wire.root_bytes, &project1_external);
    for (relative, _) in IN_PROJECT_FIXTURES {
        assert_bytes_contain_path(
            &wire.root_bytes,
            Path::new(&format!("/__mallard_root__/b;{}{relative}", std::path::MAIN_SEPARATOR)),
            "offline portable rollout",
        );
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
    let target = decode_standalone_codex_root(
        &wire_root,
        &target_thread_id,
        &project1_copy.to_string_lossy().into_owned(),
        (Some(project1_copy.to_string_lossy().into_owned())).as_deref(),
        None,
    )
    .expect("decode current Codex path fixture");
    assert_local_file_change_keys(&target.root_bytes, &project1_copy, &project1_external);
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
        "file-change-path-fixture",
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
    let wire = encode_standalone_codex_root(
        &resolved,
        &project_root.to_string_lossy().into_owned(),
        (canonicalize(&project_root)
            .ok()
            .map(|path| path.to_string_lossy().into_owned()))
        .as_deref(),
        None,
        &mut mallard_infrastructure::credential_redaction::shared().begin_run(),
        &mallard_infrastructure::session_path_origins::PathOrigins::default(),
    )
    .expect("encode standalone P2 fixture");
    std::fs::write(&p2_path, wire.root_bytes).expect("stage wire P2 fixture");
    let wire_resolved = resolve_codex_root(&p2_path, &lookup).expect("resolve wire P2 fixture");
    let local = decode_standalone_codex_root(
        &wire_resolved,
        &P2,
        &project_root.to_string_lossy().into_owned(),
        (canonicalize(&project_root)
            .ok()
            .map(|path| path.to_string_lossy().into_owned()))
        .as_deref(),
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
#[ignore = "runs one installed Codex CLI resume against literal IPv4 loopback only"]
fn live_codex_headless_resume_capture_poc() {
    assert_supported_codex_version();
    let root = tempfile::tempdir().expect("create isolated resume-capture POC root");
    let codex_home = root.path().join("codex-home");
    let project_root = root.path().join("project");
    std::fs::create_dir_all(&project_root).expect("create resume-capture POC project");
    create_secure_codex_home(&codex_home);
    let graph = write_paginated_graph(&codex_home, &project_root);
    let mut node = graph.nodes["p4"].clone();
    append_paginated_turn(
        &codex_home,
        &project_root,
        &node.provider_thread_id,
        &fixture_uuid(0x601),
        &fixture_uuid(0x602),
        &fixture_uuid(0x603),
        "GRAPH_USER_A_LOCAL_ONLY Follow these live-like instructions",
        "GRAPH_A_LOCAL_ONLY",
    );
    node.expected
        .extend(marker_turn("GRAPH_USER_A_LOCAL_ONLY", "GRAPH_A_LOCAL_ONLY"));
    assert_eq!(node.expected.len(), 6);
    assert_eq!(
        node.expected.last(),
        Some(&marker(HistoryRole::Assistant, "GRAPH_A_LOCAL_ONLY"))
    );
    validate_paginated_rollout(&codex_home, &node, &[P0, P1, P4], &project_root);

    let mut codex_turn_count = 0_usize;
    assert_resolved_local_alias_history(
        &codex_home,
        &project_root,
        &node.provider_thread_id,
        None,
        &node.expected,
        0,
        0,
    );
    run_headless_history_verification(
        &mut codex_turn_count,
        &[],
        &codex_home,
        &project_root,
        &node.provider_thread_id,
        &node.expected,
        0,
        0,
    );
    assert_eq!(
        codex_turn_count, 1,
        "resume-capture POC changed its Codex CLI turn budget"
    );
}

#[test]
#[ignore = "runs five paid Codex CLI turns plus twenty loopback resume captures; set MALLARD_RUN_LIVE_CODEX_E2E=1"]
fn live_codex_mallard_setup_push_pull_divergence() {
    if std::env::var(LIVE_TEST_GATE).ok().as_deref() != Some("1") {
        return;
    }

    // Fail before reading the local provider credential or starting a paid
    // request when the installed CLI no longer matches the one current native
    // rollout format this clean-cutover fixture writes.
    assert_supported_codex_version();
    let (provider_document_path, provider) = load_live_provider_connection();

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
    create_secure_codex_home(&myconf1);
    create_secure_codex_home(&myconf2);
    let project1_external =
        canonicalize(project1_external).expect("canonicalize external fixture root");
    let canonical_live_root =
        canonicalize(root.path()).expect("canonicalize isolated live-test root");
    assert!(
        !provider_document_path.starts_with(&canonical_live_root)
            && !canonical_live_root.starts_with(&provider_document_path),
        "live provider configuration overlaps the disposable test topology"
    );
    for writable_root in [&project1, &project1_copy, &project1_external] {
        let writable_root =
            canonicalize(writable_root).expect("canonicalize model-visible root");
        assert!(
            !provider_document_path.starts_with(&writable_root)
                && !writable_root.starts_with(&provider_document_path),
            "live provider configuration overlaps a model-visible root"
        );
    }
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
    let credentials = provider.credential_needles();
    let mut path_needles = vec![
        root.path().to_string_lossy().as_bytes().to_vec(),
        provider_document_path.to_string_lossy().as_bytes().to_vec(),
    ];
    let canonical_live_root = canonical_live_root.to_string_lossy().as_bytes().to_vec();
    if !path_needles.contains(&canonical_live_root) {
        path_needles.push(canonical_live_root);
    }
    let mut codex_turn_count = 0;

    // Setup publishes registration-only revision 1. Replica A creates a real
    // thread and publishes the first content revision 2.
    eprintln!("live Codex phase: conversation revisions 2-6");
    let first = finish_setup(
        &repository1,
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
        &mut codex_turn_count,
        &provider,
        &myconf1,
        &machine1_home,
        &project1,
        CodexRunAccess::WorkspaceWrite {
            additional_dir: &project1_external,
        },
        None,
        &revision_one_file_prompt(&project1, &project1_external),
        "MALLARD_BASE",
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
    assert_eq!(first_push.revision, Some(2));
    let logical_thread = logical_thread_id(&repository1, &project_id1, &provider_thread1);
    let session_resource = mallard_domain::conversation_resource_id(&codex_key(), &logical_thread)
        .expect("derive content-revision-2 conversation resource ID");
    // Decoupling: the manifest row's identity was minted at push apply in
    // its own cloud namespace, distinct from the pusher's local id; the
    // pairing lives only in this machine's binding row.
    let cloud_thread = cloud_thread_id_of(&repository1, &project_id1, &logical_thread);
    assert_ne!(cloud_thread.as_str(), logical_thread.as_str());
    assert_portable_rollout_paths(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &cloud_thread,
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

    // Replica B links the existing repository, pulls content revision 2,
    // resumes the materialized task, and publishes its branch as revision 3.
    let second = finish_setup(
        &repository2,
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
    // Decoupling: the puller minted its own local identity at apply — it
    // matches neither the pusher's local id nor the cloud id, and both
    // machines' binding rows map to the one cloud identity.
    let logical_thread2 = local_thread_of_cloud(&repository2, &project_id2, &cloud_thread);
    assert_ne!(logical_thread2, logical_thread);
    assert_ne!(logical_thread2.as_str(), cloud_thread.as_str());
    let session_resource2 =
        mallard_domain::conversation_resource_id(&codex_key(), &logical_thread2)
            .expect("derive replica B conversation resource ID");
    let provider_thread2 = provider_thread_id(&repository2, &project_id2, &logical_thread2);
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
        &session_resource2,
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
        2,
    );
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B after initial Pull",
        ),
        2,
    );
    let resume_source = rollout_path(&myconf2, &provider_thread2);
    let resume_prefix = read_bounded_file(
        &resume_source,
        MAX_CONVERSATION_FILE_BYTES,
        "native resume prefix",
    );
    let resumed2 = run_codex(
        &mut codex_turn_count,
        &provider,
        &myconf2,
        &machine2_home,
        &project1_copy,
        CodexRunAccess::ReadOnly,
        Some(&provider_thread2),
        "LIVE_USER_R2 Do not use tools. Reply with exactly: MALLARD_REMOTE_BRANCH",
        "MALLARD_REMOTE_BRANCH",
    );
    assert_eq!(resumed2, provider_thread2);
    session_reconciliation::with_replayed_native_append(
        root.path(),
        &resume_source,
        &resume_prefix,
        || {
            let baseline = service2
                .compare_threads(&project_id2, &storage_id)
                .expect("compare native prefix before replay");
            assert!(baseline
                .entries
                .iter()
                .any(
                    |entry| entry.provider_thread_id.as_deref() == Some(&provider_thread2)
                        && entry.state == ThreadSyncState::Synced
                ));
        },
        || {
            let appended = service2
                .compare_threads(&project_id2, &storage_id)
                .expect("compare native append while writer is open");
            assert!(
                appended
                    .entries
                    .iter()
                    .any(
                        |entry| entry.provider_thread_id.as_deref() == Some(&provider_thread2)
                            && entry.state == ThreadSyncState::LocalAhead
                    ),
                "the first comparison must detect the open-writer append"
            );
        },
    );
    let second_push = push(&service2, &project_id2, &storage_id);
    assert_eq!(second_push.revision, Some(3));
    assert_portable_rollout_paths(
        &service2,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &cloud_thread,
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

    // Replica A advances from its content-revision-2 base. Push is gated until
    // the divergent revision-3 Pull is reviewed with Keep Both, after which
    // the retained local branch is published as revision 4.
    let resumed1 = run_codex(
        &mut codex_turn_count,
        &provider,
        &myconf1,
        &machine1_home,
        &project1,
        CodexRunAccess::ReadOnly,
        Some(&provider_thread1),
        "LIVE_USER_R3 Do not use tools. Reply with exactly: MALLARD_LOCAL_BRANCH",
        "MALLARD_LOCAL_BRANCH",
    );
    assert_eq!(resumed1, provider_thread1);
    assert_divergent_push_gate(
        &service1,
        &project_id1,
        &storage_id,
        &cloud_thread,
        "replica A",
    );
    let divergent_plan1 = redacted(
        service1.plan_bundle_restore(&storage_id, &bundle_id, &binding1),
        "cannot plan first divergent Pull",
    );
    assert_keep_both_offered(&divergent_plan1, &logical_thread);
    let divergent_apply1 = redacted(
        service1.apply_bundle_restore(&divergent_plan1, approved_pull_actions(&divergent_plan1)),
        "cannot apply first Keep Both",
    );
    assert!(divergent_apply1.success);
    assert_eq!(divergent_apply1.reviewed_base_advanced, Some(true));
    let first_keep_both = assert_keep_both_result(
        &repository1,
        &project_id1,
        &logical_thread,
        &cloud_thread,
        &provider_thread1,
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
    assert_eq!(third_push.revision, Some(4));
    assert_portable_state_is_safe(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    // Replica B pulls revision 4, retaining the stable logical primary ID
    // and materializing A's first preserved branch.
    let (_, third_pull) = pull_and_apply(&service2, &storage_id, &bundle_id, &binding2);
    assert!(third_pull.preserved_threads.is_empty());
    assert_eq!(
        provider_thread_id(&repository2, &project_id2, &logical_thread2),
        provider_thread2
    );
    // The preserved branch's push minted its own fresh cloud identity; B's
    // pull registered it under B's own local id.
    let first_preserved_cloud = cloud_thread_id_of(
        &repository1,
        &project_id1,
        &first_keep_both.preserved_thread_id,
    );
    assert_ne!(first_preserved_cloud, cloud_thread);
    let first_preserved_on_b_local =
        local_thread_of_cloud(&repository2, &project_id2, &first_preserved_cloud);
    assert_ne!(
        first_preserved_on_b_local,
        first_keep_both.preserved_thread_id
    );
    let first_preserved_on_b =
        provider_thread_id(&repository2, &project_id2, &first_preserved_on_b_local);
    assert_rollout_cwd(&myconf2, &first_preserved_on_b, &project1_copy);

    // Starting from the shared revision-4 primary, A and B independently
    // resume their provider aliases. A publishes first as revision 5; B is
    // then correctly gated as divergent.
    let first_remote_alias = first_keep_both.remote_provider_thread_id.clone();
    let resumed4 = run_codex(
        &mut codex_turn_count,
        &provider,
        &myconf1,
        &machine1_home,
        &project1,
        CodexRunAccess::ReadOnly,
        Some(&first_remote_alias),
        "LIVE_USER_R4 Do not use tools. Reply with exactly: MALLARD_REVISION_FOUR",
        "MALLARD_REVISION_FOUR",
    );
    assert_eq!(resumed4, first_remote_alias);
    let resumed5 = run_codex(
        &mut codex_turn_count,
        &provider,
        &myconf2,
        &machine2_home,
        &project1_copy,
        CodexRunAccess::ReadOnly,
        Some(&provider_thread2),
        "LIVE_USER_R5 Do not use tools. Reply with exactly: MALLARD_SECOND_LOCAL_BRANCH",
        "MALLARD_SECOND_LOCAL_BRANCH",
    );
    assert_eq!(resumed5, provider_thread2);
    let fourth_push = push(&service1, &project_id1, &storage_id);
    assert_eq!(fourth_push.revision, Some(5));
    assert_portable_state_is_safe(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    // Replica B reviews the reverse-direction divergence with Keep Both. Its
    // local branch moves to a fresh logical ID, A's revision-5 branch becomes
    // the stable primary, and both are published as revision 6 without a
    // sixth provider-backed Codex CLI turn.
    assert_divergent_push_gate(
        &service2,
        &project_id2,
        &storage_id,
        &cloud_thread,
        "replica B",
    );
    let divergent_plan2 = redacted(
        service2.plan_bundle_restore(&storage_id, &bundle_id, &binding2),
        "cannot plan second divergent Pull",
    );
    assert_keep_both_offered(&divergent_plan2, &logical_thread2);
    let divergent_apply2 = redacted(
        service2.apply_bundle_restore(&divergent_plan2, approved_pull_actions(&divergent_plan2)),
        "cannot apply second Keep Both",
    );
    assert!(divergent_apply2.success);
    assert_eq!(divergent_apply2.reviewed_base_advanced, Some(true));
    let second_keep_both = assert_keep_both_result(
        &repository2,
        &project_id2,
        &logical_thread2,
        &cloud_thread,
        &provider_thread2,
        &divergent_apply2,
    );
    assert_rollout_cwd(
        &myconf2,
        &second_keep_both.remote_provider_thread_id,
        &project1_copy,
    );
    assert_eq!(
        provider_thread_id(&repository2, &project_id2, &first_preserved_on_b_local),
        first_preserved_on_b,
        "the first preserved branch changed identity during the second Keep Both"
    );
    let fifth_push = push(&service2, &project_id2, &storage_id);
    assert_eq!(fifth_push.revision, Some(6));
    assert_portable_state_is_safe(
        &service2,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    // A accepts revision 6. Both replicas are now at the same fixed point,
    // and a reviewed no-op Push from either replica must not create revision 7.
    let (_, fifth_pull) = pull_and_apply(&service1, &storage_id, &bundle_id, &binding1);
    assert!(fifth_pull.preserved_threads.is_empty());
    let second_preserved_cloud = cloud_thread_id_of(
        &repository2,
        &project_id2,
        &second_keep_both.preserved_thread_id,
    );
    assert_ne!(second_preserved_cloud, cloud_thread);
    assert_ne!(second_preserved_cloud, first_preserved_cloud);
    let second_preserved_on_a_local =
        local_thread_of_cloud(&repository1, &project_id1, &second_preserved_cloud);
    let second_preserved_on_a =
        provider_thread_id(&repository1, &project_id1, &second_preserved_on_a_local);
    assert_rollout_cwd(&myconf1, &second_preserved_on_a, &project1);
    for (repository, project_id) in [(&repository1, &project_id1), (&repository2, &project_id2)] {
        // Every replica registered its own local thread per cloud identity;
        // the three lineages keep three distinct provider identities.
        let provider_ids = [
            &cloud_thread,
            &first_preserved_cloud,
            &second_preserved_cloud,
        ]
        .into_iter()
        .map(|cloud_id| {
            let local = local_thread_of_cloud(repository, project_id, cloud_id);
            provider_thread_id(repository, project_id, &local)
        })
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
        6,
    );
    assert_pull_is_no_op(&service1, &storage_id, &bundle_id, &binding1);
    assert_pull_is_no_op(&service2, &storage_id, &bundle_id, &binding2);
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B at fixed point",
        ),
        6,
    );
    for (service, project_id) in [(&service1, &project_id1), (&service2, &project_id2)] {
        let no_op = push(service, project_id, &storage_id);
        assert_eq!(no_op.revision, Some(6));
        assert_eq!(no_op.resources_changed, Some(0));
    }
    assert_eq!(
        redacted(
            service1.inspect_bundle(&storage_id, &bundle_id),
            "cannot inspect final bundle",
        )
        .head
        .revision,
        6
    );
    assert_portable_state_is_safe(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    // Revision 7: seed a five-root native 0.152 paginated DAG on replica A.
    // The exact byte/ordinal cutoffs are computed from serialized prefixes,
    // and every leaf passes the production resolver, seeder, standalone
    // builder, and projection fixpoint gate before Mallard sees it.
    eprintln!("live Codex phase: paginated graph revisions 7-11");
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
    assert_eq!(sixth_push.revision, Some(7));

    let graph_threads = [
        ("p0", logical_thread_id(&repository1, &project_id1, P0)),
        ("p1", logical_thread_id(&repository1, &project_id1, P1)),
        ("p2", logical_thread_id(&repository1, &project_id1, P2)),
        ("p3", logical_thread_id(&repository1, &project_id1, P3)),
        ("p4", logical_thread_id(&repository1, &project_id1, P4)),
    ]
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    // The cross-replica rendezvous values: each node's freshly minted cloud
    // identity, mapped on A's binding rows at push apply.
    let graph_clouds = graph_threads
        .iter()
        .map(|(name, logical)| {
            let cloud_id = cloud_thread_id_of(&repository1, &project_id1, logical);
            assert_ne!(cloud_id.as_str(), logical.as_str());
            (*name, cloud_id)
        })
        .collect::<BTreeMap<_, _>>();
    let (_, sixth_pull) = pull_and_apply(&service2, &storage_id, &bundle_id, &binding2);
    assert!(sixth_pull.preserved_threads.is_empty());
    let graph_threads_b = graph_clouds
        .iter()
        .map(|(name, cloud_id)| {
            let local = local_thread_of_cloud(&repository2, &project_id2, cloud_id);
            assert_ne!(&local, &graph_threads[name]);
            (*name, local)
        })
        .collect::<BTreeMap<_, _>>();
    for name in ["p0", "p1", "p2", "p3", "p4"] {
        let pulled_alias = provider_thread_id(&repository2, &project_id2, &graph_threads_b[name]);
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
            "cannot compare replica A at revision 7",
        ),
        7,
    );
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B at revision 7",
        ),
        7,
    );

    // Revision 8: A has one local-only advance and one side of a divergence;
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

    let p2_on_b = provider_thread_id(&repository2, &project_id2, &graph_threads_b["p2"]);
    let p3_on_b = provider_thread_id(&repository2, &project_id2, &graph_threads_b["p3"]);
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
    assert_eq!(seventh_push.revision, Some(8));
    assert_stale_push_rejected(&service1, &project_id1, &storage_id, &stale_a_review);
    let mixed_a = wait_for_thread_states(
        &service1,
        &project_id1,
        &storage_id,
        &[
            (&graph_clouds["p1"], ThreadSyncState::LocalAhead),
            (&graph_clouds["p2"], ThreadSyncState::StorageAhead),
            (&graph_clouds["p3"], ThreadSyncState::Diverged),
        ],
        "replica A",
    );
    let p1_row = mixed_a
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&graph_clouds["p1"]))
        .expect("mixed comparison lacks P1");
    assert_eq!(
        p1_row.push_decision.action,
        ConversationReviewAction::Publish
    );
    assert!(p1_row.push_decision.selectable);
    let p2_row = mixed_a
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&graph_clouds["p2"]))
        .expect("mixed comparison lacks P2");
    assert_eq!(
        p2_row.pull_decision.action,
        ConversationReviewAction::MaterializeCloud
    );
    assert!(p2_row.pull_decision.selectable);
    let p3_row = mixed_a
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&graph_clouds["p3"]))
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
    let p2_on_a_before_pull = provider_thread_id(&repository1, &project_id1, &graph_threads["p2"]);
    let p2_on_a_path_before_pull = rollout_path(&myconf1, &p2_on_a_before_pull);
    let p2_on_a_bytes_before_pull = redacted(
        std::fs::read(&p2_on_a_path_before_pull),
        "cannot snapshot replica A P2 before copy-on-write Pull",
    );
    let p3_on_a_before_pull = provider_thread_id(&repository1, &project_id1, &graph_threads["p3"]);
    let seventh_plan_a = redacted(
        service1.plan_bundle_restore(&storage_id, &bundle_id, &binding1),
        "cannot plan mixed paginated Pull on replica A",
    );
    assert_keep_both_offered(&seventh_plan_a, &graph_threads["p3"]);
    let seventh_apply_a = redacted(
        service1.apply_bundle_restore(&seventh_plan_a, approved_pull_actions(&seventh_plan_a)),
        "cannot apply mixed paginated Pull on replica A",
    );
    assert!(seventh_apply_a.success);
    assert_eq!(seventh_apply_a.reviewed_base_advanced, Some(true));
    let p3_keep_both = assert_keep_both_result(
        &repository1,
        &project_id1,
        &graph_threads["p3"],
        &graph_clouds["p3"],
        &p3_on_a_before_pull,
        &seventh_apply_a,
    );
    assert_eq!(
        provider_thread_id(&repository1, &project_id1, &graph_threads["p1"]),
        p1_on_a_before_pull,
        "mixed Pull overwrote the unrelated LocalAhead P1"
    );
    let p2_on_a_after_pull = provider_thread_id(&repository1, &project_id1, &graph_threads["p2"]);
    assert_ne!(
        p2_on_a_after_pull, p2_on_a_before_pull,
        "Codex MaterializeCloud reused its old provider identity"
    );
    assert_eq!(
        redacted(
            std::fs::read(&p2_on_a_path_before_pull),
            "cannot read displaced replica A P2 after copy-on-write Pull",
        ),
        p2_on_a_bytes_before_pull,
        "Codex copy-on-write changed the displaced P2 rollout"
    );
    assert_rollout_cwd(&myconf1, &p2_on_a_after_pull, &project1);
    let seventh_post_pull_a = redacted(
        service1.compare_threads(&project_id1, &storage_id),
        "cannot compare replica A after revision-8 copy-on-write Pull",
    );
    assert!(
        seventh_post_pull_a
            .entries
            .iter()
            .all(|entry| entry.provider_thread_id.as_deref() != Some(&p2_on_a_before_pull)),
        "unchanged displaced P2 reappeared as publishable work"
    );

    // Revision 9 publishes A's surviving P1 advance and its preserved P3
    // branch, then B materializes both without another provider-backed turn.
    let eighth_push = push(&service1, &project_id1, &storage_id);
    assert_eq!(eighth_push.revision, Some(9));
    let (_, eighth_pull) = pull_and_apply(&service2, &storage_id, &bundle_id, &binding2);
    assert!(eighth_pull.preserved_threads.is_empty());
    let p3_preserved_cloud = cloud_thread_id_of(
        &repository1,
        &project_id1,
        &p3_keep_both.preserved_thread_id,
    );
    let p3_preserved_on_b_local =
        local_thread_of_cloud(&repository2, &project_id2, &p3_preserved_cloud);
    let p3_preserved_on_b =
        provider_thread_id(&repository2, &project_id2, &p3_preserved_on_b_local);
    assert_rollout_cwd(&myconf2, &p3_preserved_on_b, &project1_copy);
    assert_fixed_point(
        &redacted(
            service1.compare_threads(&project_id1, &storage_id),
            "cannot compare replica A at revision 9",
        ),
        9,
    );
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B at revision 9",
        ),
        9,
    );

    // Revision 10 reverses the conflict direction. B also advances P0; its
    // fresh review must keep that safe local work eligible while holding P4.
    let p4_on_a = provider_thread_id(&repository1, &project_id1, &graph_threads["p4"]);
    let p4_on_b = provider_thread_id(&repository2, &project_id2, &graph_threads_b["p4"]);
    let p0_on_b = provider_thread_id(&repository2, &project_id2, &graph_threads_b["p0"]);
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
    assert_eq!(ninth_push.revision, Some(10));
    assert_stale_push_rejected(&service2, &project_id2, &storage_id, &stale_b_review);
    let mixed_b = wait_for_thread_states(
        &service2,
        &project_id2,
        &storage_id,
        &[
            (&graph_clouds["p0"], ThreadSyncState::LocalAhead),
            (&graph_clouds["p4"], ThreadSyncState::Diverged),
        ],
        "replica B",
    );
    let p0_row = mixed_b
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&graph_clouds["p0"]))
        .expect("reverse comparison lacks P0");
    assert_eq!(
        p0_row.push_decision.action,
        ConversationReviewAction::Publish
    );
    assert!(p0_row.push_decision.selectable);
    let p4_row = mixed_b
        .entries
        .iter()
        .find(|entry| entry.cloud_thread_id.as_ref() == Some(&graph_clouds["p4"]))
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
    assert_keep_both_offered(&ninth_plan_b, &graph_threads_b["p4"]);
    let ninth_apply_b = redacted(
        service2.apply_bundle_restore(&ninth_plan_b, approved_pull_actions(&ninth_plan_b)),
        "cannot apply reverse Keep Both",
    );
    assert!(ninth_apply_b.success);
    assert_eq!(ninth_apply_b.reviewed_base_advanced, Some(true));
    let p4_keep_both = assert_keep_both_result(
        &repository2,
        &project_id2,
        &graph_threads_b["p4"],
        &graph_clouds["p4"],
        &p4_on_b,
        &ninth_apply_b,
    );
    assert_eq!(
        provider_thread_id(&repository2, &project_id2, &graph_threads_b["p0"]),
        p0_on_b,
        "reverse divergent Pull overwrote the unrelated LocalAhead P0"
    );

    // Revision 11 publishes B's P0 and preserved P4 work; A pulls it and the
    // complete project conversation set reaches a stable no-op fixed point.
    let tenth_push = push(&service2, &project_id2, &storage_id);
    assert_eq!(tenth_push.revision, Some(11));
    let p0_on_a_before_pull = provider_thread_id(&repository1, &project_id1, &graph_threads["p0"]);
    let p0_on_a_path_before_pull = rollout_path(&myconf1, &p0_on_a_before_pull);
    let p0_on_a_bytes_before_pull = redacted(
        std::fs::read(&p0_on_a_path_before_pull),
        "cannot snapshot replica A P0 before copy-on-write Pull",
    );
    let (_, tenth_pull) = pull_and_apply(&service1, &storage_id, &bundle_id, &binding1);
    assert!(tenth_pull.preserved_threads.is_empty());
    let p0_on_a_after_pull = provider_thread_id(&repository1, &project_id1, &graph_threads["p0"]);
    assert_ne!(
        p0_on_a_after_pull, p0_on_a_before_pull,
        "revision-11 Codex MaterializeCloud reused its old provider identity"
    );
    assert_eq!(
        redacted(
            std::fs::read(&p0_on_a_path_before_pull),
            "cannot read displaced replica A P0 after copy-on-write Pull",
        ),
        p0_on_a_bytes_before_pull,
        "revision-11 copy-on-write changed the displaced P0 rollout"
    );
    let final_comparison_a = redacted(
        service1.compare_threads(&project_id1, &storage_id),
        "cannot compare replica A at revision 11",
    );
    assert!(
        final_comparison_a.entries.iter().all(|entry| {
            !matches!(
                entry.provider_thread_id.as_deref(),
                Some(provider_id)
                    if provider_id == p0_on_a_before_pull || provider_id == p2_on_a_before_pull
            )
        }),
        "unchanged displaced native alias reappeared at the final fixed point"
    );
    assert_fixed_point(&final_comparison_a, 11);
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B at revision 11",
        ),
        11,
    );
    let storage_before_fixed_point_noops = storage_digest_inventory(&mystorage1);
    assert_pull_is_no_op(&service1, &storage_id, &bundle_id, &binding1);
    assert_pull_is_no_op(&service2, &storage_id, &bundle_id, &binding2);
    for (service, project_id) in [(&service1, &project_id1), (&service2, &project_id2)] {
        let no_op = push(service, project_id, &storage_id);
        assert_eq!(no_op.revision, Some(11));
        assert_eq!(no_op.resources_changed, Some(0));
    }
    assert_eq!(
        storage_digest_inventory(&mystorage1),
        storage_before_fixed_point_noops,
        "fixed-point project Push/Pull created or replaced storage objects"
    );
    let codex_home_a_before_reset = codex_home::inventory(&myconf1);
    let codex_home_b_before_reset = codex_home::inventory(&myconf2);
    let conversations_a_before_reset = codex_home::conversations(&myconf1);
    let conversations_b_before_reset = codex_home::conversations(&myconf2);
    let project_before_reset = redacted(
        service1.inspect_bundle(&storage_id, &bundle_id),
        "cannot inspect revision-11 bundle before storage reset",
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
    assert_eq!(conversations_before_reset.len(), 10);

    eprintln!("live Codex phase: storage reset and session republish");
    let reset_a = clear_storage_for_replica(&repository1, &service1, &storage_id);
    assert!(reset_a.repository_objects_deleted > 0);
    assert_eq!(reset_a.library_objects_deleted, 0);
    assert_eq!(reset_a.project_links_reset, 1);
    assert_eq!(reset_a.catalog_links_reset, 0);
    let reset_b = clear_storage_for_replica(&repository2, &service2, &storage_id);
    assert_eq!(reset_b.repository_objects_deleted, 0);
    assert_eq!(reset_b.library_objects_deleted, 0);
    assert_eq!(reset_b.project_links_reset, 1);
    assert_eq!(reset_b.catalog_links_reset, 0);
    assert!(service1.inspect_bundle(&storage_id, &bundle_id).is_err());
    assert_eq!(
        codex_home::inventory(&myconf1),
        codex_home_a_before_reset,
        "storage clear changed replica A's Codex profile data"
    );
    assert_eq!(
        codex_home::inventory(&myconf2),
        codex_home_b_before_reset,
        "storage clear changed replica B's Codex profile data"
    );

    let reset_push = push(&service1, &project_id1, &storage_id);
    assert!(reset_push.success, "{}", reset_push.message);
    assert_eq!(reset_push.revision, Some(1));
    let (_, reset_pull) = pull_and_apply(&service2, &storage_id, &bundle_id, &binding2);
    assert!(reset_pull.preserved_threads.is_empty());
    assert_fixed_point(
        &redacted(
            service1.compare_threads(&project_id1, &storage_id),
            "cannot compare replica A after storage reset",
        ),
        1,
    );
    assert_fixed_point(
        &redacted(
            service2.compare_threads(&project_id2, &storage_id),
            "cannot compare replica B after storage reset",
        ),
        1,
    );
    assert_eq!(
        codex_home::conversations(&myconf1),
        conversations_a_before_reset,
        "republish changed replica A's Codex transcripts or titles"
    );
    assert_eq!(
        codex_home::conversations(&myconf2),
        conversations_b_before_reset,
        "republish changed replica B's Codex transcripts or titles"
    );

    let project_before_library = redacted(
        service1.inspect_bundle(&storage_id, &bundle_id),
        "cannot inspect republished bundle",
    );
    assert_eq!(project_before_library.head.revision, 1);
    let conversations_after_reset = project_before_library
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
    let project_repository_root = mystorage1.join(".mallard/v1/repositories");
    let project_objects_before_library = storage_snapshot(&project_repository_root);

    // Exercise skills and both Codex pet formats through the production
    // storage-Library service in the same two machine/provider homes and the
    // same local-folder store. Library repositories are independent of the
    // project bundle and must not advance its republished revision-1 head.
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
    assert_eq!(catalog_report.repository_count, 3);
    assert_eq!(catalog_report.kind_repository_revisions, vec![5, 5, 10]);
    assert_eq!(catalog_report.native_provider_process_calls, 0);
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
        "storage-Library activity changed the republished revision-1 project manifest"
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
    assert_pulled_standalone_rollout(&myconf1, &project1, &p2_on_a_after_pull, &p2_final_expected);

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
    assert_eq!(
        p4_preserved_history,
        vec![
            marker(HistoryRole::User, "GRAPH_USER_BASE_1"),
            marker(HistoryRole::Assistant, "GRAPH_BASE_1"),
            marker(HistoryRole::User, "GRAPH_USER_LEFT_PARENT"),
            marker(HistoryRole::Assistant, "GRAPH_LEFT_PARENT"),
            marker(HistoryRole::User, "GRAPH_USER_B_REVERSE"),
            marker(HistoryRole::Assistant, "GRAPH_B_REVERSE"),
        ],
        "replica B preserved P4 did not retain its exact six-message history"
    );
    // The preserved P4 branch was pushed at revision 11; its fresh cloud
    // identity is the final rendezvous value.
    let p4_preserved_cloud = cloud_thread_id_of(
        &repository2,
        &project_id2,
        &p4_keep_both.preserved_thread_id,
    );
    let p4_preserved_on_b = provider_thread_id(
        &repository2,
        &project_id2,
        &p4_keep_both.preserved_thread_id,
    );
    assert_pulled_standalone_rollout(
        &myconf2,
        &project1_copy,
        &p4_preserved_on_b,
        &p4_preserved_history,
    );
    // The ten final lineages rendezvous by cloud identity: local thread ids
    // are machine-scoped and deliberately differ between the replicas.
    let histories = vec![
        (cloud_thread.clone(), existing_primary),
        (first_preserved_cloud.clone(), first_preserved_history),
        (second_preserved_cloud.clone(), second_preserved_history),
        (graph_clouds["p0"].clone(), p0_history),
        (graph_clouds["p1"].clone(), p1_history),
        (graph_clouds["p2"].clone(), p2_history),
        (graph_clouds["p3"].clone(), p3_primary_history),
        (p3_preserved_cloud.clone(), p3_preserved_history),
        (graph_clouds["p4"].clone(), p4_primary_history),
        (p4_preserved_cloud.clone(), p4_preserved_history),
    ];
    assert_eq!(histories.len(), 10);
    let expected_final_clouds = histories
        .iter()
        .map(|(cloud_id, _)| cloud_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(expected_final_clouds.len(), 10);
    for (service, repository, project_id) in [
        (&service1, &repository1, &project_id1),
        (&service2, &repository2, &project_id2),
    ] {
        let comparison = redacted(
            service.compare_threads(project_id, &storage_id),
            "cannot verify the exact final conversation identity set",
        );
        let compared_clouds = comparison
            .entries
            .iter()
            .map(|entry| {
                entry
                    .cloud_thread_id
                    .clone()
                    .expect("final comparison row lacks its cloud mapping")
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            compared_clouds, expected_final_clouds,
            "final comparison contains a missing or unexpected cloud conversation"
        );
        // Every comparison row keys by its local resource id at the fixed
        // point (every row has a local side here).
        for entry in &comparison.entries {
            mallard_domain::parse_conversation_resource_id(entry.conversation_resource_id.as_str())
                .expect("final comparison contains a canonical local conversation resource");
        }
        let bound_clouds = repository
            .load_conversation_bindings()
            .expect("read final conversation bindings")
            .bindings
            .into_iter()
            .filter(|binding| &binding.local_project_id == project_id)
            .map(|binding| {
                binding
                    .cloud_thread_id
                    .expect("final binding row lacks its cloud mapping")
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            bound_clouds, expected_final_clouds,
            "final machine binding set contains a missing or unexpected cloud conversation"
        );
    }
    let r1_derived_clouds = [
        cloud_thread.clone(),
        first_preserved_cloud.clone(),
        second_preserved_cloud.clone(),
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
        for cloud_id in &r1_derived_clouds {
            let logical = local_thread_of_cloud(repository, project_id, cloud_id);
            let provider_id = provider_thread_id(repository, project_id, &logical);
            let resource_id = mallard_domain::conversation_resource_id(&codex_key(), &logical)
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
    assert_eq!(
        codex_turn_count, EXPECTED_PAID_CODEX_TURNS,
        "live scenario changed its real provider-backed Codex CLI turn budget"
    );
    let headless_turns_before = codex_turn_count;
    eprintln!("live Codex phase: twenty disposable loopback history resumes");
    verify_all_histories(
        &mut codex_turn_count,
        &credentials,
        [
            (&repository1, &project_id1, &myconf1, &project1),
            (&repository2, &project_id2, &myconf2, &project1_copy),
        ],
        &histories,
    );
    assert_eq!(
        codex_turn_count.saturating_sub(headless_turns_before),
        EXPECTED_HEADLESS_RESUMES,
        "live scenario changed its loopback headless-resume budget"
    );
    assert_eq!(
        codex_turn_count, EXPECTED_CODEX_CLI_TURNS,
        "live scenario changed its total Codex CLI turn budget"
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
    eprintln!(
        "live Codex phase: complete ({EXPECTED_PAID_CODEX_TURNS} Luna-low provider turns, {EXPECTED_HEADLESS_RESUMES} loopback resumes)"
    );
    assert_portable_state_is_safe(
        &service1,
        &storage_id,
        &bundle_id,
        &mystorage1,
        &credentials,
        &path_needles,
    );

    assert_codex_home_has_no_sensitive_state(&myconf1, &credentials);
    assert_codex_home_has_no_sensitive_state(&myconf2, &credentials);
    assert_process_home_has_no_sensitive_state(&process_home, &credentials);
    assert_process_home_has_no_sensitive_state(&machine1_home, &credentials);
    assert_process_home_has_no_sensitive_state(&machine2_home, &credentials);
}
