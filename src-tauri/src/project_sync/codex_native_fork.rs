//! Codex-native fork coordination for Pull divergence resolution.
//!
//! The provider rollout is authoritative. App-server and the native PTY are
//! only ways to ask Codex to create a child; every candidate passes the same
//! bounded filesystem, ancestry, and content verification before Mallard can
//! replace the original rollout.

use chrono::{TimeZone, Utc};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};
use walkdir::WalkDir;

use super::codex_rollout::analyze_codex_rollout_structure;
use super::conversation_codec::{analyze_codex_transcript, CodecLineage};
use super::domain::{
    CodexForkAttemptStatus, CodexForkExecutor, CodexNativeChildPin, ConversationDivergencePlan,
    ThreadId, MAX_CONVERSATION_FILE_BYTES,
};

const MAX_SESSION_FILES: usize = 20_000;
const MAX_EXISTING_CHILDREN: usize = 1_024;
const MAX_APP_SERVER_LINE_BYTES: usize = 1024 * 1024;
const MAX_APP_SERVER_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_APP_SERVER_MESSAGES: usize = 4_096;
const APP_SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const APP_SERVER_FORK_OBSERVATION_INTERVAL: Duration = Duration::from_millis(200);
const FORK_PERSIST_TIMEOUT: Duration = Duration::from_secs(10);
const LOGIN_STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const NATIVE_PTY_TIMEOUT: Duration = Duration::from_secs(60);
const NATIVE_PTY_POLL_INTERVAL: Duration = Duration::from_millis(200);
const NATIVE_PTY_STABILITY_INTERVAL: Duration = Duration::from_millis(400);
const NATIVE_PTY_COMPOSER_GRACE: Duration = Duration::from_millis(400);
const NATIVE_PTY_EXIT_GRACE: Duration = Duration::from_secs(3);
const NATIVE_PTY_INTERRUPT_GRACE: Duration = Duration::from_secs(2);
const FORK_NAME_LIMIT: usize = 180;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeCodexThread {
    pub provider_thread_id: String,
    pub transcript_path: PathBuf,
    pub transcript_sha256: String,
    pub cwd: PathBuf,
    pub forked_from_id: Option<String>,
    pub lineage_digest: String,
    pub semantic_digest: String,
    pub history_record_digest: String,
    pub history_record_count: usize,
}

#[derive(Clone, Debug)]
pub struct CodexForkExecutionRequest {
    pub codex_home: PathBuf,
    pub project_directory: PathBuf,
    pub codex_executable: PathBuf,
    pub codex_executable_sha256: String,
    pub source_provider_thread_id: String,
    pub source_transcript_sha256: String,
    pub local_lineage_digest: String,
    pub local_semantic_digest: String,
    pub pinned_local_title: Option<String>,
    pub desired_fork_name: String,
    pub available_executors: Vec<CodexForkExecutor>,
    pub pre_existing_children: Vec<CodexNativeChildPin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedNativeFork {
    pub provider_thread_id: String,
    pub transcript_path: PathBuf,
    pub transcript_sha256: String,
    pub executor: CodexForkExecutor,
    pub name_status: CodexNameStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodexNameStatus {
    Applied,
    UserOverride,
    Unchanged,
    Deferred,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct CodexThreadRenameRequest {
    pub thread_id: String,
    pub expected_current: Option<String>,
    pub desired: String,
}

#[derive(Clone, Debug)]
pub struct CodexPostCommitPresentationRequest {
    pub codex_executable: PathBuf,
    pub codex_home: PathBuf,
    pub project_directory: PathBuf,
    pub fork_rename: Option<CodexThreadRenameRequest>,
    pub original_rename: Option<CodexThreadRenameRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexPostCommitPresentationOutcome {
    pub fork_name_status: Option<CodexNameStatus>,
    pub original_name_status: CodexNameStatus,
    pub inventory_refreshed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodexForkCoordinatorOutcome {
    Verified(VerifiedNativeFork),
    AwaitingManualFork,
    AbortedBeforeInvocation {
        message: String,
    },
    RecoveryRequired {
        executor: CodexForkExecutor,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExecutorOutcome {
    Verified(VerifiedNativeFork),
    DefinitiveNoFork,
    Ambiguous(String),
}

#[derive(Debug)]
enum AppServerMessage {
    Line(Vec<u8>),
    Oversized,
    StderrOversized,
    Io,
    Eof,
}

enum AppServerRequestOutcome<T> {
    Response(Value),
    Observed(T),
}

struct AppServerClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<AppServerMessage>,
    next_id: u64,
    received_bytes: usize,
    received_messages: usize,
}

impl AppServerClient {
    fn start(executable: &Path, codex_home: &Path, project: &Path) -> Result<Self, String> {
        let mut child = build_app_server_command(executable, codex_home, project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("start Codex app-server: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Codex app-server stdin is unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Codex app-server stdout is unavailable".to_string())?;
        let (sender, receiver) = mpsc::channel();
        if let Some(stderr) = child.stderr.take() {
            let sender = sender.clone();
            thread::spawn(move || {
                let mut reader = stderr;
                let mut buffer = [0_u8; 8192];
                let mut received = 0_usize;
                let mut oversized_reported = false;
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            received = received.saturating_add(count);
                            if received > MAX_APP_SERVER_OUTPUT_BYTES && !oversized_reported {
                                oversized_reported = true;
                                let _ = sender.send(AppServerMessage::StderrOversized);
                            }
                        }
                        Err(_) => {
                            let _ = sender.send(AppServerMessage::Io);
                            break;
                        }
                    }
                }
            });
        }
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match read_bounded_line(&mut reader, &mut line, MAX_APP_SERVER_LINE_BYTES) {
                    Ok(0) => {
                        let _ = sender.send(AppServerMessage::Eof);
                        break;
                    }
                    Ok(_) => {
                        if sender.send(AppServerMessage::Line(line)).is_err() {
                            break;
                        }
                    }
                    Err(BoundedLineError::Oversized) => {
                        let _ = sender.send(AppServerMessage::Oversized);
                        break;
                    }
                    Err(BoundedLineError::Io) => {
                        let _ = sender.send(AppServerMessage::Io);
                        break;
                    }
                }
            }
        });
        let mut client = Self {
            child,
            stdin: Some(stdin),
            stdout: receiver,
            next_id: 1,
            received_bytes: 0,
            received_messages: 0,
        };
        let initialize = client.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "mallard",
                    "title": "Mallard",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
            APP_SERVER_REQUEST_TIMEOUT,
        )?;
        if initialize.get("result").is_none() {
            return Err(app_server_error("initialize", &initialize));
        }
        client.notify("initialized", json!({}))?;
        Ok(client)
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        match self.request_observing(method, params, timeout, timeout, || {
            Ok(None::<std::convert::Infallible>)
        })? {
            AppServerRequestOutcome::Response(response) => Ok(response),
            AppServerRequestOutcome::Observed(never) => match never {},
        }
    }

    fn request_observing<T>(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        observation_interval: Duration,
        mut observe: impl FnMut() -> Result<Option<T>, String>,
    ) -> Result<AppServerRequestOutcome<T>, String> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.write_json(&json!({ "method": method, "id": id, "params": params }))?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(observed) = observe()? {
                return Ok(AppServerRequestOutcome::Observed(observed));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("Codex app-server {method} timed out"));
            }
            let message = match self
                .stdout
                .recv_timeout(remaining.min(observation_interval))
            {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("Codex app-server closed during {method}"))
                }
            };
            let line = match message {
                AppServerMessage::Line(line) => line,
                AppServerMessage::Oversized => {
                    return Err("Codex app-server emitted an oversized JSONL line".to_string())
                }
                AppServerMessage::StderrOversized => {
                    return Err("Codex app-server stderr exceeded its bounded limit".to_string())
                }
                AppServerMessage::Io => {
                    return Err("Codex app-server output could not be read".to_string())
                }
                AppServerMessage::Eof => {
                    return Err(format!("Codex app-server exited during {method}"))
                }
            };
            self.received_messages = self.received_messages.saturating_add(1);
            self.received_bytes = self.received_bytes.saturating_add(line.len());
            if self.received_messages > MAX_APP_SERVER_MESSAGES
                || self.received_bytes > MAX_APP_SERVER_OUTPUT_BYTES
            {
                return Err("Codex app-server output exceeded its bounded limit".to_string());
            }
            let value: Value = serde_json::from_slice(&line)
                .map_err(|_| "Codex app-server emitted malformed JSON".to_string())?;
            if let Some(response_id) = value.get("id") {
                if response_id.as_u64() == Some(id) {
                    return Ok(AppServerRequestOutcome::Response(value));
                }
                return Err(format!(
                    "Codex app-server {method} returned an unexpected response id"
                ));
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write_json(&json!({ "method": method, "params": params }))
    }

    fn write_json(&mut self, value: &Value) -> Result<(), String> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "Codex app-server stdin is closed".to_string())?;
        serde_json::to_writer(&mut *stdin, value)
            .map_err(|error| format!("encode Codex app-server request: {error}"))?;
        stdin
            .write_all(b"\n")
            .and_then(|_| stdin.flush())
            .map_err(|error| format!("write Codex app-server request: {error}"))
    }

    fn read_thread(&mut self, thread_id: &str) -> Result<Value, String> {
        let response = self.request(
            "thread/read",
            json!({ "threadId": thread_id, "includeTurns": false }),
            APP_SERVER_REQUEST_TIMEOUT,
        )?;
        response
            .get("result")
            .and_then(|result| result.get("thread"))
            .cloned()
            .ok_or_else(|| app_server_error("thread/read", &response))
    }

    fn fork_thread_observing<T>(
        &mut self,
        thread_id: &str,
        project_directory: &Path,
        observe: impl FnMut() -> Result<Option<T>, String>,
    ) -> Result<AppServerRequestOutcome<T>, String> {
        self.request_observing(
            "thread/fork",
            app_server_fork_params(thread_id, project_directory)?,
            APP_SERVER_REQUEST_TIMEOUT,
            APP_SERVER_FORK_OBSERVATION_INTERVAL,
            observe,
        )
    }

    fn set_name(&mut self, thread_id: &str, name: &str) -> Result<(), String> {
        let response = self.request(
            "thread/name/set",
            json!({ "threadId": thread_id, "name": name }),
            APP_SERVER_REQUEST_TIMEOUT,
        )?;
        if response.get("error").is_some() || response.get("result").is_none() {
            return Err(app_server_error("thread/name/set", &response));
        }
        Ok(())
    }

    fn terminate_now(&mut self) {
        self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn build_app_server_command(executable: &Path, codex_home: &Path, project: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("app-server")
        .arg("--stdio")
        .current_dir(project)
        .env("CODEX_HOME", codex_home);
    command
}

fn app_server_fork_params(thread_id: &str, project_directory: &Path) -> Result<Value, String> {
    let cwd = project_directory
        .to_str()
        .ok_or_else(|| "project directory is not valid UTF-8".to_string())?;
    Ok(json!({ "threadId": thread_id, "cwd": cwd }))
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        self.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                _ => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

enum BoundedLineError {
    Oversized,
    Io,
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    output: &mut Vec<u8>,
    limit: usize,
) -> Result<usize, BoundedLineError> {
    loop {
        let available = reader.fill_buf().map_err(|_| BoundedLineError::Io)?;
        if available.is_empty() {
            return Ok(output.len());
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(available.len());
        if output.len().saturating_add(take) > limit {
            return Err(BoundedLineError::Oversized);
        }
        output.extend_from_slice(&available[..take]);
        reader.consume(take);
        if output.last() == Some(&b'\n') {
            while output
                .last()
                .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
            {
                output.pop();
            }
            return Ok(output.len());
        }
    }
}

fn app_server_error(method: &str, response: &Value) -> String {
    let code = response
        .pointer("/error/code")
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!("Codex app-server {method} failed ({code})")
}

pub fn build_divergence_plan(
    _source_thread_id: &ThreadId,
    source_provider_thread_id: &str,
    local_lineage_digest: &str,
    local_semantic_digest: &str,
    local_title: Option<&str>,
    cloud_title: Option<&str>,
    local_summary: Option<&str>,
    created_at: u64,
    codex_home: &Path,
    _project_directory: &Path,
) -> Result<ConversationDivergencePlan, String> {
    let codex_executable = crate::codex_binary::find_binary("codex")?;
    let codex_executable = fs::canonicalize(&codex_executable)
        .map_err(|error| format!("resolve Codex executable: {error}"))?;
    let metadata = fs::symlink_metadata(&codex_executable)
        .map_err(|error| format!("inspect Codex executable: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Codex executable is not a canonical regular file".to_string());
    }
    let executable_sha256 = sha256_file(&codex_executable, 512 * 1024 * 1024)?;
    let pinned_local_title = normalize_title(local_title);
    let pinned_cloud_title = normalize_title(cloud_title);
    let base = fork_base_title(
        pinned_local_title.as_deref(),
        pinned_cloud_title.as_deref(),
        local_summary,
        source_provider_thread_id,
    );
    let desired_fork_name = planned_fork_name(&base, created_at)?;
    let pre_existing_children = native_child_pins(codex_home, source_provider_thread_id)?;
    let available_executors = vec![
        CodexForkExecutor::AppServerStdio,
        CodexForkExecutor::NativePty,
        CodexForkExecutor::Manual,
    ];
    Ok(ConversationDivergencePlan {
        available_resolutions: vec![
            super::domain::ConversationDivergenceResolution::PreserveLocalWithNativeFork,
            super::domain::ConversationDivergenceResolution::UseCloudOnly,
        ],
        preserved_thread_id: ThreadId::parse(super::domain::generated_named_id("thread")?)?,
        source_provider_thread_id: source_provider_thread_id.to_string(),
        local_lineage_digest: local_lineage_digest.to_string(),
        local_semantic_digest: local_semantic_digest.to_string(),
        pinned_local_title,
        pinned_cloud_title: pinned_cloud_title.clone(),
        desired_fork_name,
        desired_original_name: pinned_cloud_title,
        codex_executable: codex_executable.to_string_lossy().into_owned(),
        codex_executable_sha256: executable_sha256,
        available_executors,
        pre_existing_children,
    })
}

pub fn normalize_title(value: Option<&str>) -> Option<String> {
    let value = value?;
    if value.chars().any(char::is_control) {
        return None;
    }
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn fork_base_title(
    local_title: Option<&str>,
    cloud_title: Option<&str>,
    local_summary: Option<&str>,
    source_provider_thread_id: &str,
) -> String {
    [local_title, cloud_title, local_summary]
        .into_iter()
        .filter_map(normalize_title)
        .find(|candidate| candidate != source_provider_thread_id)
        .unwrap_or_else(|| "Untitled conversation".to_string())
}

pub fn planned_fork_name(base: &str, created_at: u64) -> Result<String, String> {
    if base.chars().any(char::is_control) {
        return Err("Codex fork-name base contains control characters".to_string());
    }
    let base = normalize_title(Some(base)).unwrap_or_else(|| "Untitled conversation".to_string());
    let timestamp = Utc
        .timestamp_opt(created_at as i64, 0)
        .single()
        .ok_or_else(|| "Pull plan timestamp is out of range".to_string())?
        .format("%Y-%m-%d %H:%MZ");
    let suffix = format!(" — local before Pull · {timestamp}");
    let suffix_count = suffix.chars().count();
    if suffix_count >= FORK_NAME_LIMIT {
        return Err("Codex fork-name suffix exceeds its limit".to_string());
    }
    let base_limit = FORK_NAME_LIMIT - suffix_count;
    let base = truncate_with_ellipsis(&base, base_limit);
    Ok(format!("{base}{suffix}"))
}

fn truncate_with_ellipsis(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    if limit <= 1 {
        return "…".chars().take(limit).collect();
    }
    let mut result = value.chars().take(limit - 1).collect::<String>();
    result.push('…');
    result
}

pub fn native_child_pins(
    codex_home: &Path,
    source_provider_thread_id: &str,
) -> Result<Vec<CodexNativeChildPin>, String> {
    let mut children = scan_native_threads(codex_home)?
        .into_values()
        .filter(|thread| thread.forked_from_id.as_deref() == Some(source_provider_thread_id))
        .map(|thread| CodexNativeChildPin {
            provider_thread_id: thread.provider_thread_id,
            transcript_sha256: thread.transcript_sha256,
        })
        .collect::<Vec<_>>();
    if children.len() > MAX_EXISTING_CHILDREN {
        return Err("Codex source has too many native children to review safely".to_string());
    }
    children.sort_by(|left, right| left.provider_thread_id.cmp(&right.provider_thread_id));
    Ok(children)
}

pub fn find_native_thread(
    codex_home: &Path,
    provider_thread_id: &str,
) -> Result<NativeCodexThread, String> {
    scan_native_threads(codex_home)?
        .remove(provider_thread_id)
        .ok_or_else(|| format!("Codex thread '{provider_thread_id}' was not found"))
}

fn scan_native_threads(codex_home: &Path) -> Result<BTreeMap<String, NativeCodexThread>, String> {
    let canonical_home =
        fs::canonicalize(codex_home).map_err(|error| format!("resolve Codex home: {error}"))?;
    if canonical_home != codex_home {
        return Err("Codex home is not canonical".to_string());
    }
    let root_metadata = fs::symlink_metadata(&canonical_home)
        .map_err(|error| format!("inspect Codex home: {error}"))?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err("Codex home is not a real directory".to_string());
    }
    let mut threads = BTreeMap::new();
    let mut visited = 0_usize;
    for directory in ["sessions", "archived_sessions"] {
        let root = canonical_home.join(directory);
        if !root.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(&root)
            .map_err(|error| format!("inspect Codex {directory}: {error}"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!("Codex {directory} is not a real directory"));
        }
        for entry in WalkDir::new(&root).follow_links(false).max_depth(8) {
            let entry = entry.map_err(|error| format!("scan Codex {directory}: {error}"))?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            if !is_rollout_jsonl(entry.path()) {
                continue;
            }
            visited = visited.saturating_add(1);
            if visited > MAX_SESSION_FILES {
                return Err("Codex profile has too many session files to scan safely".to_string());
            }
            let file_metadata = entry
                .metadata()
                .map_err(|error| format!("inspect Codex rollout: {error}"))?;
            if file_metadata.len() == 0 || file_metadata.len() > MAX_CONVERSATION_FILE_BYTES {
                continue;
            }
            let canonical_path = fs::canonicalize(entry.path())
                .map_err(|error| format!("resolve Codex rollout: {error}"))?;
            if !canonical_path.starts_with(&root) {
                return Err("Codex rollout resolves outside its session tree".to_string());
            }
            let bytes = read_bounded_file(&canonical_path, MAX_CONVERSATION_FILE_BYTES)?;
            let Ok(structure) = analyze_codex_rollout_structure(&bytes, None) else {
                continue;
            };
            let analysis = analyze_codex_transcript(&bytes);
            let (lineage_digest, semantic_digest) = match analysis.lineage {
                CodecLineage::Complete {
                    lineage_digest,
                    semantic_digest,
                    ..
                } => (lineage_digest, semantic_digest),
                CodecLineage::Unknown { .. } => continue,
            };
            let (history_record_digest, history_record_count) = non_metadata_record_digest(&bytes)?;
            let thread = NativeCodexThread {
                provider_thread_id: structure.current.id.clone(),
                transcript_path: canonical_path,
                transcript_sha256: hex_digest(Sha256::digest(&bytes).as_slice()),
                cwd: PathBuf::from(&structure.current.cwd),
                forked_from_id: structure.current.forked_from_id.clone(),
                lineage_digest,
                semantic_digest,
                history_record_digest,
                history_record_count,
            };
            if threads
                .insert(thread.provider_thread_id.clone(), thread)
                .is_some()
            {
                return Err("Codex provider thread has multiple rollout files".to_string());
            }
        }
    }
    Ok(threads)
}

/// Sink for fork sub-phase timings, mirroring the Pull `on_phase` callback.
/// Diagnostics only: nothing here affects fork correctness, and the no-op
/// observer (`&mut |_, _| {}`) is always a valid argument.
pub type ForkPhaseObserver<'a> = &'a mut dyn FnMut(&str, Duration);

/// Time `work`, report it under `label`, and hand back its result.
fn timed<T>(on_phase: ForkPhaseObserver<'_>, label: &str, work: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = work();
    on_phase(label, started.elapsed());
    result
}

pub fn coordinate_native_fork(
    request: &CodexForkExecutionRequest,
    record_attempt: impl FnMut(CodexForkExecutor, CodexForkAttemptStatus) -> Result<(), String>,
    on_phase: ForkPhaseObserver<'_>,
) -> Result<CodexForkCoordinatorOutcome, String> {
    if let Err(message) = timed(on_phase, "fork: validate request", || {
        validate_execution_request(request)
    }) {
        return Ok(CodexForkCoordinatorOutcome::AbortedBeforeInvocation { message });
    }
    run_executor_sequence(
        &request.available_executors,
        record_attempt,
        |executor| match executor {
            CodexForkExecutor::AppServerStdio => try_app_server_fork(request, on_phase),
            CodexForkExecutor::NativePty => try_native_pty_fork(request, on_phase),
            CodexForkExecutor::Manual => unreachable!("manual is handled by the coordinator"),
        },
    )
}

/// Validate the reviewed filesystem and executable pins before Mallard
/// creates staging. The executor performs the only app-server idle/title
/// check immediately before invoking the fork, avoiding a redundant process.
pub fn preflight_native_fork(
    request: &CodexForkExecutionRequest,
    on_phase: ForkPhaseObserver<'_>,
) -> Result<(), String> {
    timed(on_phase, "fork: preflight", || {
        validate_execution_request(request)
    })
}

fn run_executor_sequence(
    executors: &[CodexForkExecutor],
    mut record_attempt: impl FnMut(CodexForkExecutor, CodexForkAttemptStatus) -> Result<(), String>,
    mut invoke: impl FnMut(CodexForkExecutor) -> Result<ExecutorOutcome, String>,
) -> Result<CodexForkCoordinatorOutcome, String> {
    for executor in executors {
        if let Err(message) = record_attempt(*executor, CodexForkAttemptStatus::Pending) {
            return Ok(CodexForkCoordinatorOutcome::AbortedBeforeInvocation { message });
        }
        if *executor == CodexForkExecutor::Manual {
            return Ok(CodexForkCoordinatorOutcome::AwaitingManualFork);
        }
        let outcome = match invoke(*executor) {
            Ok(outcome) => outcome,
            Err(message) => {
                let message =
                    match record_attempt(*executor, CodexForkAttemptStatus::DefinitiveNoFork) {
                        Ok(()) => message,
                        Err(error) => format!(
                        "{message}; the safe pre-invocation outcome could not be journaled: {error}"
                    ),
                    };
                return Ok(CodexForkCoordinatorOutcome::AbortedBeforeInvocation { message });
            }
        };
        match outcome {
            ExecutorOutcome::Verified(fork) => {
                record_attempt(*executor, CodexForkAttemptStatus::Verified)?;
                return Ok(CodexForkCoordinatorOutcome::Verified(fork));
            }
            ExecutorOutcome::DefinitiveNoFork => {
                if let Err(message) =
                    record_attempt(*executor, CodexForkAttemptStatus::DefinitiveNoFork)
                {
                    return Ok(CodexForkCoordinatorOutcome::AbortedBeforeInvocation { message });
                }
            }
            ExecutorOutcome::Ambiguous(message) => {
                record_attempt(*executor, CodexForkAttemptStatus::Ambiguous)?;
                return Ok(CodexForkCoordinatorOutcome::RecoveryRequired {
                    executor: *executor,
                    message,
                });
            }
        }
    }
    Ok(CodexForkCoordinatorOutcome::AwaitingManualFork)
}

pub fn verify_manual_fork(
    request: &CodexForkExecutionRequest,
    provider_thread_id: Option<&str>,
    on_phase: ForkPhaseObserver<'_>,
) -> Result<VerifiedNativeFork, String> {
    timed(on_phase, "verify: validate artifacts and source", || {
        validate_execution_artifacts_and_source(request)
    })?;
    let candidate = timed(
        on_phase,
        "verify: locate candidate",
        || match provider_thread_id {
            Some(thread_id) => {
                let candidate = find_native_thread(&request.codex_home, thread_id)?;
                if pre_existing_map(&request.pre_existing_children).contains_key(thread_id) {
                    return Err(
                        "the selected Codex fork existed before this Pull review".to_string()
                    );
                }
                Ok(candidate)
            }
            None => exact_new_child(request),
        },
    )?;
    timed(on_phase, "verify: candidate ancestry and content", || {
        verify_candidate(request, &candidate)
    })?;
    timed(on_phase, "verify: source unchanged", || {
        verify_source_unchanged(request)
    })?;
    Ok(VerifiedNativeFork {
        provider_thread_id: candidate.provider_thread_id,
        transcript_path: candidate.transcript_path,
        transcript_sha256: candidate.transcript_sha256,
        executor: CodexForkExecutor::Manual,
        name_status: CodexNameStatus::Deferred,
    })
}

pub fn verify_recorded_fork(
    request: &CodexForkExecutionRequest,
    provider_thread_id: &str,
    transcript_path: &Path,
    transcript_sha256: &str,
) -> Result<(), String> {
    validate_execution_artifacts_and_source(request)?;
    let candidate = find_native_thread(&request.codex_home, provider_thread_id)?;
    if candidate.transcript_path != transcript_path
        || candidate.transcript_sha256 != transcript_sha256
    {
        return Err("verified Codex fork changed before local preservation".to_string());
    }
    verify_candidate(request, &candidate)?;
    verify_source_unchanged(request)
}

pub fn verify_source_idle_and_title(request: &CodexForkExecutionRequest) -> Result<(), String> {
    validate_execution_request(request)?;
    if !probe_source_idle_and_title(request)? {
        return Err("Codex source thread status is unavailable".to_string());
    }
    Ok(())
}

fn probe_source_idle_and_title(request: &CodexForkExecutionRequest) -> Result<bool, String> {
    let mut client = match AppServerClient::start(
        &request.codex_executable,
        &request.codex_home,
        &request.project_directory,
    ) {
        Ok(client) => client,
        Err(_) => return Ok(false),
    };
    let source = match client.read_thread(&request.source_provider_thread_id) {
        Ok(source) => source,
        Err(_) => return Ok(false),
    };
    ensure_thread_idle(&source)?;
    if normalize_title(source.get("name").and_then(Value::as_str)) != request.pinned_local_title {
        return Err("Codex source title changed after Pull review".to_string());
    }
    verify_source_unchanged(request)?;
    Ok(true)
}

fn refresh_codex_inventory_with_client(
    client: &mut AppServerClient,
    project_cwd: &Path,
) -> Result<(), String> {
    let project_cwd = project_cwd
        .to_str()
        .ok_or_else(|| "project directory is not valid UTF-8".to_string())?;
    let response = client.request(
        "thread/list",
        json!({
            "cursor": null,
            "limit": 100,
            "cwd": project_cwd,
            "useStateDbOnly": false
        }),
        APP_SERVER_REQUEST_TIMEOUT,
    )?;
    if response.get("result").is_none() || response.get("error").is_some() {
        return Err(app_server_error("thread/list", &response));
    }
    Ok(())
}

/// Complete best-effort naming and inventory presentation through one
/// app-server process. Core Pull materialization commits before this runs.
pub fn complete_codex_post_commit_presentation(
    request: &CodexPostCommitPresentationRequest,
) -> CodexPostCommitPresentationOutcome {
    let mut client = match AppServerClient::start(
        &request.codex_executable,
        &request.codex_home,
        &request.project_directory,
    ) {
        Ok(client) => client,
        Err(error) => {
            return CodexPostCommitPresentationOutcome {
                fork_name_status: request
                    .fork_rename
                    .as_ref()
                    .map(|_| CodexNameStatus::Failed(error.clone())),
                original_name_status: if request.original_rename.is_some() {
                    CodexNameStatus::Failed(error)
                } else {
                    CodexNameStatus::Unchanged
                },
                inventory_refreshed: false,
            };
        }
    };
    let mut transport_error = None;
    let fork_name_status = request.fork_rename.as_ref().map(|rename| {
        match name_with_app_server_client(
            &mut client,
            &rename.thread_id,
            rename.expected_current.as_deref(),
            &rename.desired,
        ) {
            Ok(status) => status,
            Err(error) => {
                transport_error = Some(error.clone());
                CodexNameStatus::Failed(error)
            }
        }
    });
    let original_name_status = if request.original_rename.is_none() {
        CodexNameStatus::Unchanged
    } else if let Some(error) = transport_error.clone() {
        CodexNameStatus::Failed(format!(
            "skipped after the preserved-thread naming request failed: {error}"
        ))
    } else {
        let rename = request
            .original_rename
            .as_ref()
            .expect("checked original rename above");
        match name_with_app_server_client(
            &mut client,
            &rename.thread_id,
            rename.expected_current.as_deref(),
            &rename.desired,
        ) {
            Ok(status) => status,
            Err(error) => {
                transport_error = Some(error.clone());
                CodexNameStatus::Failed(error)
            }
        }
    };
    let inventory_refreshed = transport_error.is_none()
        && refresh_codex_inventory_with_client(&mut client, &request.project_directory).is_ok();
    CodexPostCommitPresentationOutcome {
        fork_name_status,
        original_name_status,
        inventory_refreshed,
    }
}

fn try_app_server_fork(
    request: &CodexForkExecutionRequest,
    on_phase: ForkPhaseObserver<'_>,
) -> Result<ExecutorOutcome, String> {
    let mut client = match timed(on_phase, "app-server: start", || {
        AppServerClient::start(
            &request.codex_executable,
            &request.codex_home,
            &request.project_directory,
        )
    }) {
        Ok(client) => client,
        Err(_) => return Ok(ExecutorOutcome::DefinitiveNoFork),
    };
    let source = match timed(on_phase, "app-server: read source thread", || {
        client.read_thread(&request.source_provider_thread_id)
    }) {
        Ok(thread) => thread,
        Err(_) => return Ok(ExecutorOutcome::DefinitiveNoFork),
    };
    ensure_thread_idle(&source)?;
    let current_title = source.get("name").and_then(Value::as_str);
    if normalize_title(current_title) != request.pinned_local_title {
        return Err("Codex source title changed after Pull review".to_string());
    }
    timed(on_phase, "app-server: verify source unchanged", || {
        verify_source_unchanged(request)
    })?;
    let baseline = timed(on_phase, "app-server: scan baseline", || {
        Ok::<_, String>(NativeForkBaseline {
            threads: scan_native_threads(&request.codex_home)?,
            rollout_paths: scan_rollout_paths(&request.codex_home)?,
        })
    })?;
    // Each observation tick rescans the whole profile, so the poll count is
    // the multiplier on that cost — report it alongside the elapsed time.
    let mut polls = 0_usize;
    let observation_started = Instant::now();
    let mut stable_candidate: Option<(String, String, Instant)> = None;
    let fork_outcome = match client.fork_thread_observing(
        &request.source_provider_thread_id,
        &request.project_directory,
        || {
            polls = polls.saturating_add(1);
            let observation = observe_native_pty_fork(request, &baseline)?;
            match observation {
                NativePtyObservation::NoChild => {
                    if stable_candidate.is_some() {
                        return Err(
                            "Codex app-server fork disappeared during verification".to_string()
                        );
                    }
                    Ok(None)
                }
                NativePtyObservation::Persisting => {
                    if stable_candidate.is_some() {
                        return Err("Codex app-server fork changed during verification".to_string());
                    }
                    Ok(None)
                }
                NativePtyObservation::Exact {
                    provider_thread_id,
                    transcript_sha256,
                } => {
                    let now = Instant::now();
                    let observed_at = match &stable_candidate {
                        Some((existing_id, existing_digest, observed_at))
                            if existing_id == &provider_thread_id
                                && existing_digest == &transcript_sha256 =>
                        {
                            *observed_at
                        }
                        Some(_) => {
                            return Err(
                                "Codex app-server fork identity changed during verification"
                                    .to_string(),
                            )
                        }
                        None => {
                            stable_candidate =
                                Some((provider_thread_id.clone(), transcript_sha256, now));
                            now
                        }
                    };
                    if now.saturating_duration_since(observed_at) < NATIVE_PTY_STABILITY_INTERVAL {
                        return Ok(None);
                    }
                    exact_profile_delta_candidate(request, &baseline, Some(&provider_thread_id))
                }
                NativePtyObservation::Unsafe => {
                    Err("Codex app-server changed unexpected rollout state".to_string())
                }
            }
        },
    ) {
        Ok(outcome) => {
            on_phase(
                &format!("app-server: fork and observe ({polls} profile rescans)"),
                observation_started.elapsed(),
            );
            outcome
        }
        Err(error) => {
            on_phase(
                &format!("app-server: fork and observe, failed ({polls} profile rescans)"),
                observation_started.elapsed(),
            );
            return classify_app_server_result(request, &baseline, error, false);
        }
    };
    let response = match fork_outcome {
        AppServerRequestOutcome::Observed(candidate) => {
            client.terminate_now();
            return Ok(ExecutorOutcome::Verified(VerifiedNativeFork {
                provider_thread_id: candidate.provider_thread_id,
                transcript_path: candidate.transcript_path,
                transcript_sha256: candidate.transcript_sha256,
                executor: CodexForkExecutor::AppServerStdio,
                name_status: CodexNameStatus::Deferred,
            }));
        }
        AppServerRequestOutcome::Response(response) => response,
    };
    if response.get("error").is_some() {
        return classify_app_server_result(
            request,
            &baseline,
            app_server_error("thread/fork", &response),
            true,
        );
    }
    let Some(candidate_id) = response
        .pointer("/result/thread/id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return classify_app_server_result(
            request,
            &baseline,
            "Codex app-server fork response lacked a thread id".to_string(),
            false,
        );
    };
    if let Some(parent) = response
        .pointer("/result/thread/forkedFromId")
        .and_then(Value::as_str)
    {
        if parent != request.source_provider_thread_id {
            return Ok(ExecutorOutcome::Ambiguous(
                "Codex app-server returned a fork with the wrong parent".to_string(),
            ));
        }
    }
    if let Err(error) = wait_for_candidate(&request.codex_home, &candidate_id) {
        return classify_app_server_result(request, &baseline, error, false);
    }
    let candidate = match exact_profile_delta_candidate(request, &baseline, Some(&candidate_id)) {
        Ok(Some(candidate)) => candidate,
        Ok(None) => {
            return Ok(ExecutorOutcome::Ambiguous(
                "Codex app-server fork was not persisted".to_string(),
            ))
        }
        Err(error) => return Ok(ExecutorOutcome::Ambiguous(error)),
    };
    client.terminate_now();
    Ok(ExecutorOutcome::Verified(VerifiedNativeFork {
        provider_thread_id: candidate.provider_thread_id,
        transcript_path: candidate.transcript_path,
        transcript_sha256: candidate.transcript_sha256,
        executor: CodexForkExecutor::AppServerStdio,
        name_status: CodexNameStatus::Deferred,
    }))
}

#[derive(Clone, Debug)]
struct NativeForkBaseline {
    threads: BTreeMap<String, NativeCodexThread>,
    rollout_paths: BTreeSet<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NativePtyObservation {
    NoChild,
    Persisting,
    Exact {
        provider_thread_id: String,
        transcript_sha256: String,
    },
    Unsafe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativePtyShutdown {
    Observing,
    ExitSent(Instant),
    InterruptSent(Instant),
}

#[derive(Clone, Copy, Debug)]
struct NativePtyDrainReport {
    discarded_bytes: u64,
    io_failed: bool,
}

fn try_native_pty_fork(
    request: &CodexForkExecutionRequest,
    on_phase: ForkPhaseObserver<'_>,
) -> Result<ExecutorOutcome, String> {
    timed(on_phase, "pty: validate request", || {
        validate_execution_request(request)
    })?;
    let baseline = timed(on_phase, "pty: scan baseline", || {
        Ok::<_, String>(NativeForkBaseline {
            threads: scan_native_threads(&request.codex_home)?,
            rollout_paths: scan_rollout_paths(&request.codex_home)?,
        })
    })?;
    if !timed(on_phase, "pty: probe source idle and title", || {
        probe_source_idle_and_title(request)
    })? {
        return classify_native_pty_without_spawn(
            request,
            &baseline,
            "Codex source status is unavailable for native PTY fork",
        );
    }
    if !timed(on_phase, "pty: login status preflight", || {
        codex_login_status_ready(request)
    })? {
        return classify_native_pty_without_spawn(
            request,
            &baseline,
            "Codex login status preflight did not complete successfully",
        );
    }
    let spawn_started = Instant::now();
    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: 40,
        cols: 120,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(pair) => pair,
        Err(_) => {
            return classify_native_pty_without_spawn(
                request,
                &baseline,
                "native PTY is unavailable",
            )
        }
    };
    let portable_pty::PtyPair { slave, master } = pair;
    let reader = match master.try_clone_reader() {
        Ok(reader) => reader,
        Err(_) => {
            return classify_native_pty_without_spawn(
                request,
                &baseline,
                "native PTY output could not be opened",
            )
        }
    };
    let writer = match master.take_writer() {
        Ok(writer) => writer,
        Err(_) => {
            return classify_native_pty_without_spawn(
                request,
                &baseline,
                "native PTY input could not be opened",
            )
        }
    };
    let command = build_native_pty_command(request);
    let child = match slave.spawn_command(command) {
        Ok(child) => child,
        Err(_) => {
            return classify_native_pty_without_spawn(
                request,
                &baseline,
                "Codex could not be spawned in the native PTY",
            )
        }
    };
    drop(slave);
    on_phase("pty: spawn codex fork", spawn_started.elapsed());
    let session_started = Instant::now();
    let session = run_native_pty_session(request, baseline, child, reader, writer, master);
    // The session polls a full profile rescan every NATIVE_PTY_POLL_INTERVAL
    // until it observes a stable child or hits NATIVE_PTY_TIMEOUT, so this
    // phase alone can account for the whole timeout budget.
    on_phase("pty: run session and observe", session_started.elapsed());
    match session {
        Ok(outcome) => Ok(outcome),
        Err(_) => Ok(ExecutorOutcome::Ambiguous(
            "native PTY fork completion could not be verified safely".to_string(),
        )),
    }
}

fn build_native_pty_command(request: &CodexForkExecutionRequest) -> CommandBuilder {
    let mut command = CommandBuilder::new(request.codex_executable.as_os_str());
    command.arg("fork");
    command.arg("--no-alt-screen");
    command.arg("-C");
    command.arg(request.project_directory.as_os_str());
    command.arg(request.source_provider_thread_id.as_str());
    command.cwd(request.project_directory.as_os_str());
    command.env("CODEX_HOME", request.codex_home.as_os_str());
    #[cfg(unix)]
    if std::env::var_os("TERM")
        .as_deref()
        .and_then(|term| term.to_str())
        .is_none_or(|term| term.is_empty() || term == "dumb")
    {
        command.env("TERM", "xterm-256color");
    }
    command
}

fn codex_login_status_ready(request: &CodexForkExecutionRequest) -> Result<bool, String> {
    let mut child = match Command::new(&request.codex_executable)
        .arg("login")
        .arg("status")
        .current_dir(&request.project_directory)
        .env("CODEX_HOME", &request.codex_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return Ok(false),
    };
    let deadline = Instant::now() + LOGIN_STATUS_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(false);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(false);
            }
        }
    }
}

fn classify_native_pty_without_spawn(
    request: &CodexForkExecutionRequest,
    baseline: &NativeForkBaseline,
    message: &str,
) -> Result<ExecutorOutcome, String> {
    verify_source_unchanged(request)?;
    let current_paths = scan_rollout_paths(&request.codex_home)?;
    let current_threads = scan_native_threads(&request.codex_home)?;
    if current_paths == baseline.rollout_paths && current_threads == baseline.threads {
        Ok(ExecutorOutcome::DefinitiveNoFork)
    } else {
        Ok(ExecutorOutcome::Ambiguous(message.to_string()))
    }
}

fn run_native_pty_session(
    request: &CodexForkExecutionRequest,
    baseline: NativeForkBaseline,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    mut reader: Box<dyn Read + Send>,
    mut writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
) -> Result<ExecutorOutcome, String> {
    let (drain_sender, drain_receiver) = mpsc::channel();
    let drain_worker = thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        let mut discarded_bytes = 0_u64;
        let io_failed = loop {
            match reader.read(&mut buffer) {
                Ok(0) => break false,
                Ok(count) => {
                    discarded_bytes = discarded_bytes.saturating_add(count as u64);
                }
                Err(_) => break true,
            }
        };
        let report = NativePtyDrainReport {
            discarded_bytes,
            io_failed,
        };
        let _ = drain_sender.send(report);
        report
    });

    let operation_deadline = Instant::now() + NATIVE_PTY_TIMEOUT;
    let mut shutdown = NativePtyShutdown::Observing;
    let mut stable_candidate: Option<(String, String, Instant)> = None;
    let mut lifecycle_uncertainty = None;
    let mut drain_report = None;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                if child.wait().is_err() {
                    lifecycle_uncertainty =
                        Some("native PTY child could not be reaped".to_string());
                }
                break;
            }
            Ok(None) => {}
            Err(_) => {
                lifecycle_uncertainty =
                    Some("native PTY child status became uncertain".to_string());
                let _ = terminate_native_pty_child(&mut *child);
                break;
            }
        }

        if drain_report.is_none() && drain_worker.is_finished() {
            drain_report = drain_receiver.try_recv().ok();
            lifecycle_uncertainty = Some(match drain_report {
                Some(_) => "native PTY output drain ended before Codex exited".to_string(),
                None => "native PTY output drain worker failed".to_string(),
            });
            let _ = terminate_native_pty_child(&mut *child);
            break;
        }

        let now = Instant::now();
        match shutdown {
            NativePtyShutdown::Observing => {
                let observation = match observe_native_pty_fork(request, &baseline) {
                    Ok(observation) => observation,
                    Err(_) => {
                        lifecycle_uncertainty =
                            Some("native PTY fork state could not be scanned safely".to_string());
                        let _ = terminate_native_pty_child(&mut *child);
                        break;
                    }
                };
                match observation {
                    NativePtyObservation::NoChild => {
                        stable_candidate = None;
                    }
                    NativePtyObservation::Persisting => {
                        if stable_candidate.is_some() {
                            lifecycle_uncertainty =
                                Some("native PTY fork changed while being verified".to_string());
                            let _ = terminate_native_pty_child(&mut *child);
                            break;
                        }
                    }
                    NativePtyObservation::Unsafe => {
                        lifecycle_uncertainty =
                            Some("native PTY changed unexpected Codex rollout state".to_string());
                        let _ = terminate_native_pty_child(&mut *child);
                        break;
                    }
                    NativePtyObservation::Exact {
                        provider_thread_id,
                        transcript_sha256,
                    } => {
                        let observed_at = match &stable_candidate {
                            Some((existing_id, existing_digest, observed_at))
                                if existing_id == &provider_thread_id
                                    && existing_digest == &transcript_sha256 =>
                            {
                                *observed_at
                            }
                            Some(_) => {
                                lifecycle_uncertainty = Some(
                                    "native PTY fork identity changed during verification"
                                        .to_string(),
                                );
                                let _ = terminate_native_pty_child(&mut *child);
                                break;
                            }
                            None => {
                                stable_candidate =
                                    Some((provider_thread_id, transcript_sha256, now));
                                now
                            }
                        };
                        if now.saturating_duration_since(observed_at)
                            >= NATIVE_PTY_STABILITY_INTERVAL + NATIVE_PTY_COMPOSER_GRACE
                        {
                            if writer
                                .write_all(b"/exit\r")
                                .and_then(|_| writer.flush())
                                .is_err()
                            {
                                lifecycle_uncertainty = Some(
                                    "native PTY could not request a graceful exit".to_string(),
                                );
                                let _ = terminate_native_pty_child(&mut *child);
                                break;
                            }
                            shutdown = NativePtyShutdown::ExitSent(now + NATIVE_PTY_EXIT_GRACE);
                        }
                    }
                }
                if now >= operation_deadline {
                    lifecycle_uncertainty =
                        Some("native PTY fork exceeded its bounded runtime".to_string());
                    if writer
                        .write_all(&[0x03])
                        .and_then(|_| writer.flush())
                        .is_err()
                    {
                        let _ = terminate_native_pty_child(&mut *child);
                        break;
                    }
                    shutdown = NativePtyShutdown::InterruptSent(now + NATIVE_PTY_INTERRUPT_GRACE);
                }
            }
            NativePtyShutdown::ExitSent(deadline) if now >= deadline => {
                if writer
                    .write_all(&[0x03])
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    lifecycle_uncertainty = Some(
                        "native PTY could not interrupt Codex after graceful exit".to_string(),
                    );
                    let _ = terminate_native_pty_child(&mut *child);
                    break;
                }
                shutdown = NativePtyShutdown::InterruptSent(now + NATIVE_PTY_INTERRUPT_GRACE);
            }
            NativePtyShutdown::InterruptSent(deadline) if now >= deadline => {
                if !terminate_native_pty_child(&mut *child) {
                    lifecycle_uncertainty =
                        Some("native PTY Codex process could not be reaped".to_string());
                }
                break;
            }
            NativePtyShutdown::ExitSent(_) | NativePtyShutdown::InterruptSent(_) => {}
        }
        thread::sleep(NATIVE_PTY_POLL_INTERVAL);
    }

    drop(writer);
    drop(master);
    match drain_worker.join() {
        Ok(report) => {
            debug_assert!(drain_report.is_none_or(|early| {
                early.discarded_bytes == report.discarded_bytes
                    && early.io_failed == report.io_failed
            }));
            let _discarded_bytes = report.discarded_bytes;
            if report.io_failed {
                lifecycle_uncertainty =
                    Some("native PTY output drain failed before cleanup".to_string());
            }
        }
        Err(_) => {
            lifecycle_uncertainty = Some("native PTY output drain worker failed".to_string());
        }
    }

    classify_native_pty_completion(request, &baseline, lifecycle_uncertainty)
}

fn terminate_native_pty_child(child: &mut (dyn portable_pty::Child + Send + Sync)) -> bool {
    match child.kill() {
        Ok(()) => child.wait().is_ok(),
        Err(_) => match child.try_wait() {
            Ok(Some(_)) => child.wait().is_ok(),
            _ => false,
        },
    }
}

fn observe_native_pty_fork(
    request: &CodexForkExecutionRequest,
    baseline: &NativeForkBaseline,
) -> Result<NativePtyObservation, String> {
    let current_paths = scan_rollout_paths(&request.codex_home)?;
    if !baseline.rollout_paths.is_subset(&current_paths) {
        return Ok(NativePtyObservation::Unsafe);
    }
    let new_paths = current_paths
        .difference(&baseline.rollout_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if new_paths.len() > 1 {
        return Ok(NativePtyObservation::Unsafe);
    }
    let current_threads = scan_native_threads(&request.codex_home)?;
    if baseline.threads.iter().any(|(thread_id, before)| {
        current_threads.get(thread_id).is_none_or(|after| {
            before.transcript_path != after.transcript_path
                || before.transcript_sha256 != after.transcript_sha256
        })
    }) {
        return Ok(NativePtyObservation::Unsafe);
    }
    let mut new_threads = current_threads
        .values()
        .filter(|thread| !baseline.threads.contains_key(&thread.provider_thread_id));
    let candidate = new_threads.next();
    if new_threads.next().is_some() {
        return Ok(NativePtyObservation::Unsafe);
    }
    let Some(candidate) = candidate else {
        return Ok(if new_paths.is_empty() {
            NativePtyObservation::NoChild
        } else {
            NativePtyObservation::Persisting
        });
    };
    if new_paths.len() != 1 || !new_paths.contains(&candidate.transcript_path) {
        return Ok(NativePtyObservation::Unsafe);
    }
    if verify_candidate(request, candidate).is_err() || verify_source_unchanged(request).is_err() {
        return Ok(NativePtyObservation::Unsafe);
    }
    Ok(NativePtyObservation::Exact {
        provider_thread_id: candidate.provider_thread_id.clone(),
        transcript_sha256: candidate.transcript_sha256.clone(),
    })
}

fn classify_native_pty_completion(
    request: &CodexForkExecutionRequest,
    baseline: &NativeForkBaseline,
    lifecycle_uncertainty: Option<String>,
) -> Result<ExecutorOutcome, String> {
    let current_paths = scan_rollout_paths(&request.codex_home)?;
    let current_threads = scan_native_threads(&request.codex_home)?;
    let existing_changed = baseline
        .rollout_paths
        .iter()
        .any(|path| !current_paths.contains(path))
        || baseline.threads.iter().any(|(thread_id, before)| {
            current_threads.get(thread_id).is_none_or(|after| {
                before.transcript_path != after.transcript_path
                    || before.transcript_sha256 != after.transcript_sha256
            })
        });
    let new_paths = current_paths
        .difference(&baseline.rollout_paths)
        .collect::<BTreeSet<_>>();
    let new_threads = current_threads
        .values()
        .filter(|thread| !baseline.threads.contains_key(&thread.provider_thread_id))
        .collect::<Vec<_>>();
    if existing_changed || new_threads.len() > 1 || new_paths.len() > 1 {
        return Ok(ExecutorOutcome::Ambiguous(
            lifecycle_uncertainty
                .unwrap_or_else(|| "native PTY changed unexpected Codex rollout state".to_string()),
        ));
    }
    if let Some(candidate) = new_threads.first() {
        if new_paths.len() != 1
            || !new_paths.contains(&candidate.transcript_path)
            || verify_candidate(request, candidate).is_err()
            || verify_source_unchanged(request).is_err()
        {
            return Ok(ExecutorOutcome::Ambiguous(
                "native PTY fork failed final ancestry or content verification".to_string(),
            ));
        }
        return Ok(ExecutorOutcome::Verified(VerifiedNativeFork {
            provider_thread_id: candidate.provider_thread_id.clone(),
            transcript_path: candidate.transcript_path.clone(),
            transcript_sha256: candidate.transcript_sha256.clone(),
            executor: CodexForkExecutor::NativePty,
            name_status: CodexNameStatus::Deferred,
        }));
    }
    if !new_paths.is_empty() {
        return Ok(ExecutorOutcome::Ambiguous(
            "native PTY left an unverified Codex rollout".to_string(),
        ));
    }
    match lifecycle_uncertainty {
        Some(message) => Ok(ExecutorOutcome::Ambiguous(message)),
        None => Ok(ExecutorOutcome::DefinitiveNoFork),
    }
}

fn scan_rollout_paths(codex_home: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let canonical_home =
        fs::canonicalize(codex_home).map_err(|error| format!("resolve Codex home: {error}"))?;
    if canonical_home != codex_home {
        return Err("Codex home is not canonical".to_string());
    }
    let mut paths = BTreeSet::new();
    let mut visited = 0_usize;
    for directory in ["sessions", "archived_sessions"] {
        let root = canonical_home.join(directory);
        if !root.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(&root)
            .map_err(|error| format!("inspect Codex {directory}: {error}"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!("Codex {directory} is not a real directory"));
        }
        for entry in WalkDir::new(&root).follow_links(false).max_depth(8) {
            let entry = entry.map_err(|error| format!("scan Codex {directory}: {error}"))?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            if !is_rollout_jsonl(entry.path()) {
                continue;
            }
            visited = visited.saturating_add(1);
            if visited > MAX_SESSION_FILES {
                return Err("Codex profile has too many session files to scan safely".to_string());
            }
            let canonical_path = fs::canonicalize(entry.path())
                .map_err(|error| format!("resolve Codex rollout: {error}"))?;
            if !canonical_path.starts_with(&root) {
                return Err("Codex rollout resolves outside its session tree".to_string());
            }
            paths.insert(canonical_path);
        }
    }
    Ok(paths)
}

fn classify_app_server_result(
    request: &CodexForkExecutionRequest,
    baseline: &NativeForkBaseline,
    error: String,
    definitively_rejected: bool,
) -> Result<ExecutorOutcome, String> {
    match exact_profile_delta_candidate(request, baseline, None) {
        Ok(Some(candidate)) => Ok(ExecutorOutcome::Verified(VerifiedNativeFork {
            provider_thread_id: candidate.provider_thread_id,
            transcript_path: candidate.transcript_path,
            transcript_sha256: candidate.transcript_sha256,
            executor: CodexForkExecutor::AppServerStdio,
            name_status: CodexNameStatus::Deferred,
        })),
        Ok(None) if definitively_rejected => Ok(ExecutorOutcome::DefinitiveNoFork),
        Ok(None) | Err(_) => Ok(ExecutorOutcome::Ambiguous(error)),
    }
}

fn exact_profile_delta_candidate(
    request: &CodexForkExecutionRequest,
    baseline: &NativeForkBaseline,
    expected_provider_thread_id: Option<&str>,
) -> Result<Option<NativeCodexThread>, String> {
    let current_paths = scan_rollout_paths(&request.codex_home)?;
    let current_threads = scan_native_threads(&request.codex_home)?;
    if !baseline.rollout_paths.is_subset(&current_paths)
        || baseline.threads.iter().any(|(thread_id, before)| {
            current_threads.get(thread_id).is_none_or(|after| {
                before.transcript_path != after.transcript_path
                    || before.transcript_sha256 != after.transcript_sha256
            })
        })
    {
        return Err("Codex rollout state changed during native fork".to_string());
    }
    let new_paths = current_paths
        .difference(&baseline.rollout_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    let new_threads = current_threads
        .values()
        .filter(|thread| !baseline.threads.contains_key(&thread.provider_thread_id))
        .collect::<Vec<_>>();
    if new_paths.is_empty() && new_threads.is_empty() {
        return Ok(None);
    }
    if new_paths.len() != 1 || new_threads.len() != 1 {
        return Err("native fork changed unexpected Codex rollout state".to_string());
    }
    let candidate = new_threads[0];
    if !new_paths.contains(&candidate.transcript_path)
        || expected_provider_thread_id
            .is_some_and(|expected| candidate.provider_thread_id != expected)
    {
        return Err("native fork returned an unexpected Codex thread".to_string());
    }
    verify_candidate(request, candidate)?;
    verify_source_unchanged(request)?;
    Ok(Some(candidate.clone()))
}

fn validate_execution_request(request: &CodexForkExecutionRequest) -> Result<(), String> {
    validate_execution_artifacts_and_source(request)?;
    let current_children =
        native_child_pins(&request.codex_home, &request.source_provider_thread_id)?;
    let expected = pre_existing_map(&request.pre_existing_children);
    if current_children
        .iter()
        .any(|child| expected.get(&child.provider_thread_id) != Some(&child.transcript_sha256))
        || current_children.len() != expected.len()
    {
        return Err("Codex native children changed after Pull review".to_string());
    }
    Ok(())
}

fn validate_execution_artifacts_and_source(
    request: &CodexForkExecutionRequest,
) -> Result<(), String> {
    for path in [
        &request.codex_home,
        &request.project_directory,
        &request.codex_executable,
    ] {
        if !path.is_absolute() {
            return Err("Codex fork execution path is not absolute".to_string());
        }
    }
    validate_canonical_directory(&request.codex_home, "Codex home")?;
    validate_canonical_directory(&request.project_directory, "project directory")?;
    validate_canonical_file(&request.codex_executable, "Codex executable")?;
    if sha256_file(&request.codex_executable, 512 * 1024 * 1024)? != request.codex_executable_sha256
    {
        return Err("Codex executable changed after Pull review".to_string());
    }
    verify_source_unchanged(request)?;
    Ok(())
}

fn validate_canonical_directory(path: &Path, label: &str) -> Result<(), String> {
    let canonical = fs::canonicalize(path).map_err(|error| format!("resolve {label}: {error}"))?;
    if canonical != path {
        return Err(format!("{label} is not canonical"));
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect {label}: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("{label} is not a real directory"));
    }
    Ok(())
}

fn validate_canonical_file(path: &Path, label: &str) -> Result<(), String> {
    let canonical = fs::canonicalize(path).map_err(|error| format!("resolve {label}: {error}"))?;
    if canonical != path {
        return Err(format!("{label} is not canonical"));
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect {label}: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{label} is not a real file"));
    }
    Ok(())
}

fn is_rollout_jsonl(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
}

fn ensure_thread_idle(thread: &Value) -> Result<(), String> {
    let status = thread
        .pointer("/status/type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match status {
        "idle" | "notLoaded" => Ok(()),
        "active" => Err("Codex source thread has an active turn".to_string()),
        "systemError" => Err("Codex source thread is in a system-error state".to_string()),
        _ => Err("Codex source thread status could not be verified".to_string()),
    }
}

fn verify_source_unchanged(request: &CodexForkExecutionRequest) -> Result<(), String> {
    let source = find_native_thread(&request.codex_home, &request.source_provider_thread_id)?;
    if source.transcript_sha256 != request.source_transcript_sha256
        || source.lineage_digest != request.local_lineage_digest
        || source.semantic_digest != request.local_semantic_digest
    {
        return Err("Codex source thread changed after Pull review".to_string());
    }
    Ok(())
}

fn verify_candidate(
    request: &CodexForkExecutionRequest,
    candidate: &NativeCodexThread,
) -> Result<(), String> {
    verify_candidate_workspace(request, candidate)?;
    let source = find_native_thread(&request.codex_home, &request.source_provider_thread_id)?;
    if candidate.provider_thread_id == request.source_provider_thread_id
        || candidate.forked_from_id.as_deref() != Some(&request.source_provider_thread_id)
        || candidate.lineage_digest != request.local_lineage_digest
        || candidate.semantic_digest != request.local_semantic_digest
        || candidate.history_record_digest != source.history_record_digest
        || candidate.history_record_count != source.history_record_count
        || pre_existing_map(&request.pre_existing_children)
            .contains_key(&candidate.provider_thread_id)
    {
        return Err("Codex fork candidate failed ancestry or content verification".to_string());
    }
    let before = pre_existing_map(&request.pre_existing_children);
    let current = native_child_pins(&request.codex_home, &request.source_provider_thread_id)?
        .into_iter()
        .map(|child| (child.provider_thread_id, child.transcript_sha256))
        .collect::<BTreeMap<_, _>>();
    if current.len() != before.len().saturating_add(1)
        || current.get(&candidate.provider_thread_id) != Some(&candidate.transcript_sha256)
        || before
            .iter()
            .any(|(thread_id, digest)| current.get(thread_id) != Some(digest))
    {
        return Err("Codex native child inventory changed during fork verification".to_string());
    }
    Ok(())
}

fn verify_candidate_workspace(
    request: &CodexForkExecutionRequest,
    candidate: &NativeCodexThread,
) -> Result<(), String> {
    let canonical_cwd = fs::canonicalize(&candidate.cwd)
        .map_err(|_| "Codex fork candidate cwd could not be resolved".to_string())?;
    let metadata = fs::symlink_metadata(&canonical_cwd)
        .map_err(|_| "Codex fork candidate cwd could not be inspected".to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Codex fork candidate cwd is not a real directory".to_string());
    }
    if canonical_cwd != request.project_directory
        && !canonical_cwd.starts_with(&request.project_directory)
    {
        return Err("Codex fork candidate cwd is outside the reviewed project".to_string());
    }
    Ok(())
}

/// Records a faithful fork does not reproduce byte for byte.
/// `session_meta` carries the fork's own identity and parent pointer, while
/// `turn_context` and `event_msg/thread_settings_applied` carry environment
/// that the app re-emits under its current schema. We have observed those
/// environment records drop default fields such as `missing_path_behavior`
/// while describing the same policy. None is conversation content, and the
/// message chain is pinned separately by the lineage and semantic digests.
const FORK_VOLATILE_RECORD_TYPES: [&str; 2] = ["session_meta", "turn_context"];

fn is_fork_volatile_record(value: &Value) -> bool {
    if super::codex_rollout::structural_record_type(value)
        .is_some_and(|record_type| FORK_VOLATILE_RECORD_TYPES.contains(&record_type))
    {
        return true;
    }
    value.get("type").and_then(Value::as_str) == Some("event_msg")
        && value.pointer("/payload/type").and_then(Value::as_str) == Some("thread_settings_applied")
}

/// Digest over the records a fork must carry over unchanged.  Comparing it
/// against the source is what proves a candidate preserves the same
/// conversation, so it must tolerate exactly what Codex rewrites when forking
/// and nothing more.
fn non_metadata_record_digest(bytes: &[u8]) -> Result<(String, usize), String> {
    let mut hasher = Sha256::new();
    let mut count = 0_usize;
    for raw in bytes.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_slice(raw)
            .map_err(|_| "Codex rollout contains malformed JSON".to_string())?;
        if is_fork_volatile_record(&value) {
            continue;
        }
        // Codex restamps every record it copies into a fork.  Only the record
        // envelope is dropped; timestamps inside the payload stay hashed.
        if let Some(object) = value.as_object_mut() {
            object.remove("timestamp");
        }
        let canonical = serde_json::to_vec(&value)
            .map_err(|_| "Codex rollout record could not be canonicalized".to_string())?;
        hasher.update((canonical.len() as u64).to_be_bytes());
        hasher.update(canonical);
        count = count.saturating_add(1);
    }
    Ok((hex_digest(hasher.finalize().as_slice()), count))
}

fn wait_for_candidate(
    codex_home: &Path,
    provider_thread_id: &str,
) -> Result<NativeCodexThread, String> {
    let deadline = Instant::now() + FORK_PERSIST_TIMEOUT;
    loop {
        match find_native_thread(codex_home, provider_thread_id) {
            Ok(candidate) => return Ok(candidate),
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Err(error) => return Err(error),
        }
    }
}

fn exact_new_child(request: &CodexForkExecutionRequest) -> Result<NativeCodexThread, String> {
    let mut delta = new_children(request)?;
    if delta.len() != 1 {
        return Err(if delta.is_empty() {
            "no new Codex native fork was found".to_string()
        } else {
            "multiple new Codex native forks require recovery".to_string()
        });
    }
    Ok(delta.remove(0))
}

fn new_children(request: &CodexForkExecutionRequest) -> Result<Vec<NativeCodexThread>, String> {
    let before = pre_existing_map(&request.pre_existing_children);
    let mut children = scan_native_threads(&request.codex_home)?
        .into_values()
        .filter(|thread| {
            thread.forked_from_id.as_deref() == Some(&request.source_provider_thread_id)
                && !before.contains_key(&thread.provider_thread_id)
        })
        .collect::<Vec<_>>();
    children.sort_by(|left, right| left.provider_thread_id.cmp(&right.provider_thread_id));
    Ok(children)
}

fn pre_existing_map(children: &[CodexNativeChildPin]) -> BTreeMap<String, String> {
    children
        .iter()
        .map(|child| {
            (
                child.provider_thread_id.clone(),
                child.transcript_sha256.clone(),
            )
        })
        .collect()
}

fn name_with_app_server_client(
    client: &mut AppServerClient,
    thread_id: &str,
    expected_current: Option<&str>,
    desired: &str,
) -> Result<CodexNameStatus, String> {
    let thread = client.read_thread(thread_id)?;
    let raw_current = thread.get("name").and_then(Value::as_str);
    let current = normalize_title(raw_current);
    if raw_current.is_some() && current.is_none() {
        return Err("Codex thread has an invalid current name".to_string());
    }
    if current.as_deref() == Some(desired) {
        return Ok(CodexNameStatus::Unchanged);
    }
    if current.is_some() && current.as_deref() != expected_current {
        return Ok(CodexNameStatus::UserOverride);
    }
    client.set_name(thread_id, desired)?;
    Ok(CodexNameStatus::Applied)
}

fn read_bounded_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect bounded Codex file: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
        return Err("Codex file is not a bounded regular file".to_string());
    }
    let file = File::open(path).map_err(|error| format!("open bounded Codex file: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read bounded Codex file: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err("Codex file exceeds its read limit".to_string());
    }
    Ok(bytes)
}

fn sha256_file(path: &Path, limit: u64) -> Result<String, String> {
    let bytes = read_bounded_file(path, limit)?;
    Ok(hex_digest(Sha256::digest(bytes).as_slice()))
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::ffi::OsString;
    use std::io::IsTerminal;

    fn test_rollout(thread_id: &str, parent: Option<&str>, cwd: &Path) -> Vec<u8> {
        test_rollout_with_parent_metadata(thread_id, parent, cwd, None)
    }

    fn test_rollout_with_parent_metadata(
        thread_id: &str,
        parent: Option<&str>,
        cwd: &Path,
        parent_cwd: Option<&Path>,
    ) -> Vec<u8> {
        let mut records = vec![json!({
            "timestamp": "2026-07-29T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": thread_id,
                "forked_from_id": parent,
                "cwd": cwd.to_str().unwrap(),
            },
        })];
        if let (Some(parent), Some(parent_cwd)) = (parent, parent_cwd) {
            records.push(json!({
                "timestamp": "2026-07-29T00:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": parent,
                    "forked_from_id": null,
                    "cwd": parent_cwd.to_str().unwrap(),
                },
            }));
        }
        records.extend([
            json!({
                "timestamp": "2026-07-29T00:00:01Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "id": "m1",
                    "parent_id": null,
                    "role": "user",
                    "content": [{"type": "input_text", "text": "question"}],
                },
            }),
            json!({
                "timestamp": "2026-07-29T00:00:02Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "id": "m2",
                    "parent_id": "m1",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "answer"}],
                },
            }),
        ]);
        let mut bytes = records
            .into_iter()
            .map(|record| record.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        bytes.push(b'\n');
        bytes
    }

    fn write_test_rollout(
        home: &Path,
        thread_id: &str,
        parent: Option<&str>,
        cwd: &Path,
    ) -> PathBuf {
        let directory = home.join("sessions/2026/07/29");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("rollout-{thread_id}.jsonl"));
        fs::write(&path, test_rollout(thread_id, parent, cwd)).unwrap();
        path
    }

    fn manual_test_request(root: &Path, source_id: &str) -> (CodexForkExecutionRequest, PathBuf) {
        let home = root.join("codex");
        let project = root.join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&project).unwrap();
        let home = fs::canonicalize(home).unwrap();
        let project = fs::canonicalize(project).unwrap();
        write_test_rollout(&home, source_id, None, &project);
        let executable = root.join("codex-test-bin");
        fs::write(&executable, b"not an executable").unwrap();
        let executable = fs::canonicalize(executable).unwrap();
        let source = find_native_thread(&home, source_id).unwrap();
        (
            CodexForkExecutionRequest {
                codex_home: home.clone(),
                project_directory: project,
                codex_executable: executable.clone(),
                codex_executable_sha256: sha256_file(&executable, 1024).unwrap(),
                source_provider_thread_id: source_id.to_string(),
                source_transcript_sha256: source.transcript_sha256,
                local_lineage_digest: source.lineage_digest,
                local_semantic_digest: source.semantic_digest,
                pinned_local_title: None,
                desired_fork_name: "Question — local before Pull · 2026-07-29 18:42Z".to_string(),
                available_executors: vec![CodexForkExecutor::Manual],
                pre_existing_children: Vec::new(),
            },
            home,
        )
    }

    #[test]
    fn planned_name_is_normalized_bounded_and_stable() {
        let name =
            planned_fork_name(&format!("  {}   graph  ", "界".repeat(200)), 1_775_000_000).unwrap();
        assert_eq!(name.chars().count(), FORK_NAME_LIMIT);
        assert!(name.contains("… — local before Pull · "));
        assert!(name.ends_with("Z"));
        assert!(!name.contains("  "));
        assert_eq!(
            name,
            planned_fork_name(&format!("  {}   graph  ", "界".repeat(200)), 1_775_000_000,)
                .unwrap()
        );
    }

    #[test]
    fn planned_name_rejects_control_characters() {
        let error = planned_fork_name("Unsafe\nname", 1_775_000_000).unwrap_err();
        assert!(error.contains("control characters"), "{error}");
        assert_eq!(normalize_title(Some("Unsafe\nname")), None);
    }

    #[test]
    fn fork_base_title_uses_reviewed_priority_and_never_uses_provider_identity() {
        assert_eq!(
            fork_base_title(
                Some("Local title"),
                Some("Cloud title"),
                Some("First request"),
                "provider-id",
            ),
            "Local title"
        );
        assert_eq!(
            fork_base_title(
                Some("provider-id"),
                Some("Cloud title"),
                Some("First request"),
                "provider-id",
            ),
            "Cloud title"
        );
        assert_eq!(
            fork_base_title(None, None, Some("  First   request  "), "provider-id"),
            "First request"
        );
        assert_eq!(
            fork_base_title(None, None, Some("provider-id"), "provider-id"),
            "Untitled conversation"
        );
    }

    #[test]
    fn native_pty_command_uses_exact_argv_cwd_and_reviewed_home() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let (mut request, _) = manual_test_request(temp.path(), source_id);
        request.project_directory = PathBuf::from("/tmp/project with spaces/知识图");
        request.codex_home = PathBuf::from("/tmp/codex home/配置");

        let command = build_native_pty_command(&request);
        assert_eq!(
            command.get_argv(),
            &vec![
                request.codex_executable.as_os_str().to_owned(),
                "fork".into(),
                "--no-alt-screen".into(),
                "-C".into(),
                request.project_directory.as_os_str().to_owned(),
                source_id.into(),
            ]
        );
        assert_eq!(
            command.get_cwd().map(OsString::as_os_str),
            Some(request.project_directory.as_os_str())
        );
        assert_eq!(
            command.get_env("CODEX_HOME"),
            Some(request.codex_home.as_os_str())
        );
        assert!(!command
            .get_argv()
            .iter()
            .any(|argument| matches!(argument.to_str(), Some("sh" | "cmd.exe" | "powershell"))));
    }

    #[test]
    fn app_server_fork_params_include_the_reviewed_project_cwd() {
        let project = Path::new("/tmp/project with spaces/知识图");
        assert_eq!(
            app_server_fork_params("thread-a", project).unwrap(),
            json!({
                "threadId": "thread-a",
                "cwd": "/tmp/project with spaces/知识图",
            })
        );
    }

    #[test]
    fn app_server_command_uses_stdio_project_cwd_and_reviewed_home() {
        let executable = Path::new("/opt/codex/bin/codex");
        let codex_home = Path::new("/tmp/myconf");
        let project = Path::new("/tmp/HorrorGame");
        let command = build_app_server_command(executable, codex_home, project);

        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec!["app-server", "--stdio"]
        );
        assert_eq!(command.get_current_dir(), Some(project));
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "CODEX_HOME")
                .and_then(|(_, value)| value),
            Some(codex_home.as_os_str())
        );
    }

    #[cfg(unix)]
    #[test]
    fn app_server_adopts_a_verified_rollout_before_the_fork_response() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let (mut request, home) = manual_test_request(temp.path(), source_id);
        fs::write(
            home.join("fork-template.jsonl"),
            test_rollout(child_id, Some(source_id), &request.project_directory),
        )
        .unwrap();
        let script = r#"#!/bin/sh
while IFS= read -r line
do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"id":1,"result":{}}'
      ;;
    *'"method":"thread/read"'*)
      printf '%s\n' '{"id":2,"result":{"thread":{"id":"__SOURCE_ID__","name":null,"status":{"type":"idle"}}}}'
      ;;
    *'"method":"thread/fork"'*)
      cp "$CODEX_HOME/fork-template.jsonl" "$CODEX_HOME/sessions/2026/07/29/rollout-__CHILD_ID__.jsonl"
      ;;
  esac
done
"#
        .replace("__SOURCE_ID__", source_id)
        .replace("__CHILD_ID__", child_id);
        let executable = temp.path().join("fake-codex");
        fs::write(&executable, script).unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).unwrap();
        let executable = fs::canonicalize(executable).unwrap();
        request.codex_executable = executable.clone();
        request.codex_executable_sha256 = sha256_file(&executable, 512 * 1024 * 1024).unwrap();

        let started = Instant::now();
        let outcome = try_app_server_fork(&request, &mut |_, _| {}).unwrap();

        let ExecutorOutcome::Verified(verified) = outcome else {
            panic!("persisted fork was not adopted");
        };
        assert_eq!(verified.provider_thread_id, child_id);
        assert_eq!(verified.executor, CodexForkExecutor::AppServerStdio);
        assert_eq!(verified.name_status, CodexNameStatus::Deferred);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "fork waited for the missing app-server response"
        );
    }

    #[cfg(unix)]
    #[test]
    fn post_commit_presentation_reuses_one_app_server_process() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let project = temp.path().join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&project).unwrap();
        let home = fs::canonicalize(home).unwrap();
        let project = fs::canonicalize(project).unwrap();
        let script = r#"#!/bin/sh
printf '%s\n' start >> "$CODEX_HOME/app-server-starts"
read_count=0
name_count=0
while IFS= read -r line
do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"id":1,"result":{}}'
      ;;
    *'"method":"thread/read"'*)
      read_count=$((read_count + 1))
      if [ "$read_count" -eq 1 ]; then response_id=2; else response_id=4; fi
      printf '{"id":%s,"result":{"thread":{"name":null}}}\n' "$response_id"
      ;;
    *'"method":"thread/name/set"'*)
      name_count=$((name_count + 1))
      if [ "$name_count" -eq 1 ]; then response_id=3; else response_id=5; fi
      printf '{"id":%s,"result":{}}\n' "$response_id"
      ;;
    *'"method":"thread/list"'*)
      printf '%s\n' '{"id":6,"result":{"data":[],"nextCursor":null}}'
      ;;
  esac
done
"#;
        let executable = temp.path().join("fake-codex");
        fs::write(&executable, script).unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).unwrap();
        let executable = fs::canonicalize(executable).unwrap();
        let request = CodexPostCommitPresentationRequest {
            codex_executable: executable,
            codex_home: home.clone(),
            project_directory: project,
            fork_rename: Some(CodexThreadRenameRequest {
                thread_id: "fork".to_string(),
                expected_current: None,
                desired: "Preserved local".to_string(),
            }),
            original_rename: Some(CodexThreadRenameRequest {
                thread_id: "original".to_string(),
                expected_current: None,
                desired: "Cloud".to_string(),
            }),
        };

        let outcome = complete_codex_post_commit_presentation(&request);

        assert_eq!(outcome.fork_name_status, Some(CodexNameStatus::Applied));
        assert_eq!(outcome.original_name_status, CodexNameStatus::Applied);
        assert!(outcome.inventory_refreshed);
        assert_eq!(
            fs::read_to_string(home.join("app-server-starts")).unwrap(),
            "start\n"
        );
    }

    #[test]
    fn bounded_line_reader_rejects_oversized_jsonl() {
        let bytes = vec![b'x'; MAX_APP_SERVER_LINE_BYTES + 1];
        let mut reader = BufReader::new(bytes.as_slice());
        let mut output = Vec::new();
        assert!(matches!(
            read_bounded_line(&mut reader, &mut output, MAX_APP_SERVER_LINE_BYTES),
            Err(BoundedLineError::Oversized)
        ));
    }

    #[test]
    fn manual_verifier_accepts_only_one_exact_new_native_child() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let (request, home) = manual_test_request(temp.path(), source_id);
        write_test_rollout(&home, child_id, Some(source_id), &request.project_directory);

        let verified = verify_manual_fork(&request, Some(child_id), &mut |_, _| {}).unwrap();
        assert_eq!(verified.provider_thread_id, child_id);
        assert_eq!(verified.executor, CodexForkExecutor::Manual);
        assert_eq!(verified.name_status, CodexNameStatus::Deferred);
    }

    #[test]
    fn manual_verifier_accepts_a_child_workspace_below_the_project_root() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let (request, home) = manual_test_request(temp.path(), source_id);
        let child_cwd = request.project_directory.join("apps/web");
        fs::create_dir_all(&child_cwd).unwrap();
        let child_cwd = fs::canonicalize(child_cwd).unwrap();
        write_test_rollout(&home, child_id, Some(source_id), &child_cwd);

        let verified = verify_manual_fork(&request, Some(child_id), &mut |_, _| {}).unwrap();
        assert_eq!(verified.provider_thread_id, child_id);
        assert_eq!(find_native_thread(&home, child_id).unwrap().cwd, child_cwd);
    }

    #[test]
    fn app_server_bad_workspace_child_requires_recovery_instead_of_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let (request, home) = manual_test_request(temp.path(), source_id);
        let baseline = NativeForkBaseline {
            threads: scan_native_threads(&request.codex_home).unwrap(),
            rollout_paths: scan_rollout_paths(&request.codex_home).unwrap(),
        };

        assert_eq!(
            classify_app_server_result(
                &request,
                &baseline,
                "app-server rejected the fork".to_string(),
                true,
            )
            .unwrap(),
            ExecutorOutcome::DefinitiveNoFork
        );

        let outside_cwd = temp.path().join("project-elsewhere");
        fs::create_dir_all(&outside_cwd).unwrap();
        let outside_cwd = fs::canonicalize(outside_cwd).unwrap();
        let directory = home.join("sessions/2026/07/29");
        let candidate_path = directory.join(format!("rollout-{child_id}.jsonl"));
        fs::write(
            candidate_path,
            test_rollout_with_parent_metadata(
                child_id,
                Some(source_id),
                &outside_cwd,
                Some(&request.project_directory),
            ),
        )
        .unwrap();

        let candidate = find_native_thread(&home, child_id).unwrap();
        assert_eq!(candidate.cwd, outside_cwd);
        let error = exact_profile_delta_candidate(&request, &baseline, Some(child_id)).unwrap_err();
        assert!(error.contains("outside the reviewed project"), "{error}");
        assert_eq!(
            classify_app_server_result(
                &request,
                &baseline,
                "app-server rejected the fork".to_string(),
                true,
            )
            .unwrap(),
            ExecutorOutcome::Ambiguous("app-server rejected the fork".to_string())
        );
    }

    #[test]
    fn manual_verifier_rejects_multiple_new_children_without_guessing() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let other_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644d";
        let (request, home) = manual_test_request(temp.path(), source_id);
        write_test_rollout(&home, child_id, Some(source_id), &request.project_directory);
        write_test_rollout(&home, other_id, Some(source_id), &request.project_directory);

        let error = verify_manual_fork(&request, Some(child_id), &mut |_, _| {}).unwrap_err();
        assert!(error.contains("inventory changed"), "{error}");
    }

    #[test]
    fn native_pty_final_scan_accepts_one_exact_child_even_after_lifecycle_uncertainty() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let (request, home) = manual_test_request(temp.path(), source_id);
        let baseline = NativeForkBaseline {
            threads: scan_native_threads(&request.codex_home).unwrap(),
            rollout_paths: scan_rollout_paths(&request.codex_home).unwrap(),
        };
        write_test_rollout(&home, child_id, Some(source_id), &request.project_directory);

        let outcome = classify_native_pty_completion(
            &request,
            &baseline,
            Some("forced termination".to_string()),
        )
        .unwrap();
        let ExecutorOutcome::Verified(verified) = outcome else {
            panic!("expected an exact fork to be adopted");
        };
        assert_eq!(verified.provider_thread_id, child_id);
        assert_eq!(verified.executor, CodexForkExecutor::NativePty);
    }

    #[test]
    fn native_pty_final_scan_rejects_an_unverified_new_rollout() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let (request, home) = manual_test_request(temp.path(), source_id);
        let baseline = NativeForkBaseline {
            threads: scan_native_threads(&request.codex_home).unwrap(),
            rollout_paths: scan_rollout_paths(&request.codex_home).unwrap(),
        };
        let directory = home.join("sessions/2026/07/29");
        fs::write(directory.join("rollout-partial.jsonl"), b"{").unwrap();

        let outcome = classify_native_pty_completion(&request, &baseline, None).unwrap();
        assert!(matches!(outcome, ExecutorOutcome::Ambiguous(_)));
    }

    #[test]
    fn native_pty_pre_spawn_failure_requires_an_unchanged_complete_profile() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let (request, home) = manual_test_request(temp.path(), source_id);
        let baseline = NativeForkBaseline {
            threads: scan_native_threads(&request.codex_home).unwrap(),
            rollout_paths: scan_rollout_paths(&request.codex_home).unwrap(),
        };

        assert_eq!(
            classify_native_pty_without_spawn(&request, &baseline, "PTY unavailable").unwrap(),
            ExecutorOutcome::DefinitiveNoFork
        );

        let directory = home.join("sessions/2026/07/29");
        fs::write(directory.join("rollout-incomplete.jsonl"), b"{").unwrap();
        assert_eq!(
            classify_native_pty_without_spawn(&request, &baseline, "PTY unavailable").unwrap(),
            ExecutorOutcome::Ambiguous("PTY unavailable".to_string())
        );
    }

    #[test]
    fn rollout_scan_ignores_non_rollout_jsonl_files() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let (request, home) = manual_test_request(temp.path(), source_id);
        let before = scan_rollout_paths(&request.codex_home).unwrap();
        fs::write(
            home.join("sessions/2026/07/29/unrelated.jsonl"),
            test_rollout(
                "019fac73-c1ff-79b3-bb8e-1673ed0d644c",
                Some(source_id),
                &request.project_directory,
            ),
        )
        .unwrap();

        assert_eq!(scan_rollout_paths(&request.codex_home).unwrap(), before);
        assert_eq!(
            scan_native_threads(&request.codex_home)
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec![source_id.to_string()]
        );
    }

    #[test]
    fn fake_codex_pty_subprocess_entrypoint() {
        if std::env::var_os("MALLARD_TEST_CODEX_PTY_CHILD").is_none() {
            return;
        }
        assert!(std::io::stdin().is_terminal());
        assert!(std::io::stdout().is_terminal());
        let home = PathBuf::from(
            std::env::var_os("MALLARD_TEST_CODEX_HOME")
                .expect("fake Codex home is provided by the parent test"),
        );
        let source_id = std::env::var("MALLARD_TEST_CODEX_SOURCE")
            .expect("fake Codex source is provided by the parent test");
        let child_id = std::env::var("MALLARD_TEST_CODEX_CHILD")
            .expect("fake Codex child is provided by the parent test");
        let project = PathBuf::from(
            std::env::var_os("MALLARD_TEST_CODEX_PROJECT")
                .expect("fake Codex project is provided by the parent test"),
        );
        write_test_rollout(&home, &child_id, Some(&source_id), &project);

        let sentinel = b"MALLARD-PTY-OUTPUT-MUST-NOT-BE-RETAINED";
        let mut stdout = std::io::stdout().lock();
        for _ in 0..32_768 {
            stdout.write_all(sentinel).unwrap();
        }
        stdout.flush().unwrap();
        drop(stdout);

        let mut stdin = std::io::stdin().lock();
        let mut received = Vec::new();
        let mut byte = [0_u8; 1];
        while stdin.read(&mut byte).unwrap_or(0) == 1 {
            received.push(byte[0]);
            if received.ends_with(b"/exit\r") || received.ends_with(b"/exit\n") || byte[0] == 0x03 {
                break;
            }
            if received.len() > 64 {
                received.drain(..received.len() - 64);
            }
        }
    }

    #[test]
    fn native_pty_drains_output_and_exits_one_exact_fake_codex_child() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let (mut request, home) = manual_test_request(temp.path(), source_id);
        let executable = std::env::current_exe().unwrap();
        request.codex_executable = executable.clone();
        request.codex_executable_sha256 = sha256_file(&executable, 512 * 1024 * 1024).unwrap();
        let baseline = NativeForkBaseline {
            threads: scan_native_threads(&request.codex_home).unwrap(),
            rollout_paths: scan_rollout_paths(&request.codex_home).unwrap(),
        };
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let portable_pty::PtyPair { slave, master } = pair;
        let reader = master.try_clone_reader().unwrap();
        let writer = master.take_writer().unwrap();
        let mut command = CommandBuilder::new(executable);
        command.arg("--exact");
        command.arg("project_sync::codex_native_fork::tests::fake_codex_pty_subprocess_entrypoint");
        command.arg("--nocapture");
        command.cwd(&request.project_directory);
        command.env("MALLARD_TEST_CODEX_PTY_CHILD", "1");
        command.env("MALLARD_TEST_CODEX_HOME", &home);
        command.env("MALLARD_TEST_CODEX_PROJECT", &request.project_directory);
        command.env("MALLARD_TEST_CODEX_SOURCE", source_id);
        command.env("MALLARD_TEST_CODEX_CHILD", child_id);
        let child = slave.spawn_command(command).unwrap();
        drop(slave);

        let outcome =
            run_native_pty_session(&request, baseline, child, reader, writer, master).unwrap();
        let debug = format!("{outcome:?}");
        let ExecutorOutcome::Verified(verified) = outcome else {
            panic!("fake Codex PTY did not produce an exact verified child: {debug}");
        };
        assert_eq!(verified.provider_thread_id, child_id);
        assert_eq!(verified.executor, CodexForkExecutor::NativePty);
        assert!(!debug.contains("MALLARD-PTY-OUTPUT-MUST-NOT-BE-RETAINED"));
    }

    fn test_verified_fork(executor: CodexForkExecutor) -> VerifiedNativeFork {
        VerifiedNativeFork {
            provider_thread_id: "fork-id".to_string(),
            transcript_path: PathBuf::from("/tmp/fork.jsonl"),
            transcript_sha256: "a".repeat(64),
            executor,
            name_status: CodexNameStatus::Applied,
        }
    }

    #[test]
    fn executor_sequence_stops_after_stdio_success() {
        let mut invoked = Vec::new();
        let mut attempts = Vec::new();
        let outcome = run_executor_sequence(
            &[
                CodexForkExecutor::AppServerStdio,
                CodexForkExecutor::NativePty,
                CodexForkExecutor::Manual,
            ],
            |executor, status| {
                attempts.push((executor, status));
                Ok(())
            },
            |executor| {
                invoked.push(executor);
                Ok(ExecutorOutcome::Verified(test_verified_fork(executor)))
            },
        )
        .unwrap();
        assert_eq!(invoked, vec![CodexForkExecutor::AppServerStdio]);
        assert_eq!(
            attempts,
            vec![
                (
                    CodexForkExecutor::AppServerStdio,
                    CodexForkAttemptStatus::Pending,
                ),
                (
                    CodexForkExecutor::AppServerStdio,
                    CodexForkAttemptStatus::Verified,
                ),
            ]
        );
        assert!(matches!(outcome, CodexForkCoordinatorOutcome::Verified(_)));
    }

    #[test]
    fn executor_sequence_falls_through_only_after_definitive_no_fork() {
        let mut invoked = Vec::new();
        let outcome = run_executor_sequence(
            &[
                CodexForkExecutor::AppServerStdio,
                CodexForkExecutor::NativePty,
                CodexForkExecutor::Manual,
            ],
            |_, _| Ok(()),
            |executor| {
                invoked.push(executor);
                Ok(ExecutorOutcome::DefinitiveNoFork)
            },
        )
        .unwrap();
        assert_eq!(
            invoked,
            vec![
                CodexForkExecutor::AppServerStdio,
                CodexForkExecutor::NativePty,
            ]
        );
        assert_eq!(outcome, CodexForkCoordinatorOutcome::AwaitingManualFork);
    }

    #[test]
    fn executor_sequence_never_falls_through_after_ambiguity() {
        let mut invoked = Vec::new();
        let outcome = run_executor_sequence(
            &[
                CodexForkExecutor::AppServerStdio,
                CodexForkExecutor::NativePty,
                CodexForkExecutor::Manual,
            ],
            |_, _| Ok(()),
            |executor| {
                invoked.push(executor);
                Ok(ExecutorOutcome::Ambiguous("uncertain".to_string()))
            },
        )
        .unwrap();
        assert_eq!(invoked, vec![CodexForkExecutor::AppServerStdio]);
        assert_eq!(
            outcome,
            CodexForkCoordinatorOutcome::RecoveryRequired {
                executor: CodexForkExecutor::AppServerStdio,
                message: "uncertain".to_string(),
            }
        );
    }

    #[test]
    fn executor_sequence_aborts_and_closes_pending_attempt_before_invocation_error() {
        let mut invoked = Vec::new();
        let mut attempts = Vec::new();
        let outcome = run_executor_sequence(
            &[
                CodexForkExecutor::AppServerStdio,
                CodexForkExecutor::NativePty,
                CodexForkExecutor::Manual,
            ],
            |executor, status| {
                attempts.push((executor, status));
                Ok(())
            },
            |executor| {
                invoked.push(executor);
                Err("source became active".to_string())
            },
        )
        .unwrap();
        assert_eq!(invoked, vec![CodexForkExecutor::AppServerStdio]);
        assert_eq!(
            attempts,
            vec![
                (
                    CodexForkExecutor::AppServerStdio,
                    CodexForkAttemptStatus::Pending,
                ),
                (
                    CodexForkExecutor::AppServerStdio,
                    CodexForkAttemptStatus::DefinitiveNoFork,
                ),
            ]
        );
        assert_eq!(
            outcome,
            CodexForkCoordinatorOutcome::AbortedBeforeInvocation {
                message: "source became active".to_string(),
            }
        );
    }
}
