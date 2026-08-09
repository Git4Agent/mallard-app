//! Codex-native fork coordination for Pull divergence resolution.
//!
//! The provider rollout is authoritative. App-server and the native PTY are
//! only ways to ask Codex to create a child; every candidate passes the same
//! bounded filesystem, ancestry, and content verification before Mallard can
//! replace the original rollout.

use chrono::{TimeZone, Utc};
use notify::event::{ModifyKind, RenameMode};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::codex_rollout::analyze_codex_rollout_structure;
use crate::conversation_codec::{analyze_codex_transcript, CodecLineage};
use mallard_domain::{
    CodexForkAttemptStatus, CodexForkExecutor, ConversationDivergencePlan, ForkOrigin, ThreadId,
    MAX_CONVERSATION_FILE_BYTES,
};

const MAX_APP_SERVER_LINE_BYTES: usize = 1024 * 1024;
const MAX_APP_SERVER_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_APP_SERVER_MESSAGES: usize = 4_096;
const APP_SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const LOGIN_STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const NATIVE_PTY_TIMEOUT: Duration = Duration::from_secs(60);
const NATIVE_PTY_EVENT_INTERVAL: Duration = Duration::from_millis(100);
const NATIVE_PTY_STABILITY_INTERVAL: Duration = Duration::from_millis(400);
const NATIVE_PTY_COMPOSER_GRACE: Duration = Duration::from_millis(400);
const NATIVE_PTY_EXIT_OBSERVATION_GRACE: Duration = Duration::from_secs(2);
const NATIVE_PTY_EXIT_GRACE: Duration = Duration::from_secs(3);
const NATIVE_PTY_INTERRUPT_GRACE: Duration = Duration::from_secs(2);
const NATIVE_PTY_DRAIN_GRACE: Duration = Duration::from_secs(1);
const NATIVE_PTY_WATCH_QUEUE: usize = 256;
const NATIVE_PTY_MAX_CANDIDATES: usize = 16;
const NATIVE_PTY_MAX_EVENT_PATHS: usize = 64;
const NATIVE_PTY_MAX_VERIFY_RETRIES: u8 = 8;
const NATIVE_PTY_VERIFY_RETRY_BASE: Duration = Duration::from_millis(100);
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
    /// Exact machine-local rollout path captured during review and persisted
    /// in the operation journal. Fork execution never discovers it by scanning.
    pub source_transcript_path: PathBuf,
    pub source_transcript_sha256: String,
    pub local_lineage_digest: String,
    pub local_semantic_digest: String,
    pub pinned_local_title: Option<String>,
    pub desired_fork_name: String,
    pub available_executors: Vec<CodexForkExecutor>,
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
    AbortedBeforeInvocation {
        message: String,
    },
    Unacknowledged {
        executor: CodexForkExecutor,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExecutorOutcome {
    Verified(VerifiedNativeFork),
    DefinitiveNoFork,
    Unacknowledged(String),
}

#[derive(Debug)]
enum AppServerMessage {
    Line(Vec<u8>),
    Oversized,
    StderrOversized,
    Io,
    Eof,
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
                },
                "capabilities": {
                    "experimentalApi": true
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
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.write_json(&json!({ "method": method, "id": id, "params": params }))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("Codex app-server {method} timed out"));
            }
            let message = match self.stdout.recv_timeout(remaining) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("Codex app-server {method} timed out"))
                }
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
                    return Ok(value);
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

    fn fork_thread(&mut self, thread_id: &str, project_directory: &Path) -> Result<Value, String> {
        self.request(
            "thread/fork",
            app_server_fork_params(thread_id, project_directory)?,
            APP_SERVER_REQUEST_TIMEOUT,
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
    Ok(json!({
        "threadId": thread_id,
        "cwd": cwd,
        "excludeTurns": true
    }))
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

// Every argument is one pinned plan field; a params struct would only
// restate the ConversationDivergencePlan it builds.
#[allow(clippy::too_many_arguments)]
pub fn build_divergence_plan(
    source_provider_thread_id: &str,
    local_lineage_digest: &str,
    local_semantic_digest: &str,
    local_message_count: u64,
    local_branch_preserved: bool,
    origin: ForkOrigin,
    local_title: Option<&str>,
    cloud_title: Option<&str>,
    local_summary: Option<&str>,
    created_at: u64,
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
    let desired_fork_name = planned_fork_name(&base, created_at, origin)?;
    let available_executors = vec![
        CodexForkExecutor::AppServerStdio,
        CodexForkExecutor::NativePty,
    ];
    let recommended_resolution = if local_branch_preserved {
        mallard_domain::ConversationDivergenceResolution::UseCloudOnly
    } else {
        mallard_domain::ConversationDivergenceResolution::PreserveLocalWithNativeFork
    };
    Ok(ConversationDivergencePlan {
        available_resolutions: vec![
            mallard_domain::ConversationDivergenceResolution::PreserveLocalWithNativeFork,
            mallard_domain::ConversationDivergenceResolution::UseCloudOnly,
        ],
        recommended_resolution,
        local_branch_preserved,
        preserved_thread_id: ThreadId::parse(mallard_domain::generated_named_id("thread")?)?,
        source_provider_thread_id: source_provider_thread_id.to_string(),
        local_lineage_digest: local_lineage_digest.to_string(),
        local_semantic_digest: local_semantic_digest.to_string(),
        local_message_count,
        pinned_local_title,
        pinned_cloud_title: pinned_cloud_title.clone(),
        desired_fork_name,
        desired_original_name: match origin {
            ForkOrigin::PullPreserve => pinned_cloud_title,
            ForkOrigin::Manual => None,
        },
        execution: mallard_domain::DivergenceExecution::CodexNativeFork {
            codex_executable: codex_executable.to_string_lossy().into_owned(),
            codex_executable_sha256: executable_sha256,
            available_executors,
        },
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

pub fn planned_fork_name(
    base: &str,
    created_at: u64,
    origin: ForkOrigin,
) -> Result<String, String> {
    if base.chars().any(char::is_control) {
        return Err("Codex fork-name base contains control characters".to_string());
    }
    let base = normalize_title(Some(base)).unwrap_or_else(|| "Untitled conversation".to_string());
    let timestamp = Utc
        .timestamp_opt(created_at as i64, 0)
        .single()
        .ok_or_else(|| "fork plan timestamp is out of range".to_string())?
        .format("%Y-%m-%d %H:%MZ");
    let suffix = match origin {
        ForkOrigin::PullPreserve => format!(" — local before Pull · {timestamp}"),
        ForkOrigin::Manual => format!(" — fork · {timestamp}"),
    };
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

pub(crate) fn native_thread_at_path(
    codex_home: &Path,
    transcript_path: &Path,
    expected_provider_thread_id: &str,
) -> Result<NativeCodexThread, String> {
    let thread = native_thread_from_path(codex_home, transcript_path)?;
    if thread.provider_thread_id != expected_provider_thread_id {
        return Err("Codex rollout identity does not match the expected thread".to_string());
    }
    Ok(thread)
}

pub fn verify_native_thread_prefix_at_path(
    codex_home: &Path,
    transcript_path: &Path,
    expected_provider_thread_id: &str,
    expected_message_count: usize,
    expected_lineage_digest: &str,
    expected_semantic_digest: &str,
) -> Result<(), String> {
    let (thread, bytes) = native_thread_with_bytes_from_path(codex_home, transcript_path)?;
    if thread.provider_thread_id != expected_provider_thread_id {
        return Err("Codex rollout identity does not match the expected thread".to_string());
    }
    if thread.lineage_digest == expected_lineage_digest
        && thread.semantic_digest == expected_semantic_digest
    {
        return Ok(());
    }
    let analysis = analyze_codex_transcript(&bytes);
    let chain = analysis
        .lineage
        .chain()
        .ok_or_else(|| "Codex rollout lineage is no longer readable".to_string())?;
    if chain.len() < expected_message_count {
        return Err("Codex rollout no longer contains the expected prefix".to_string());
    }
    let (prefix_lineage, prefix_semantic) =
        mallard_domain::conversation_chain_digests(&chain[..expected_message_count])?;
    if prefix_lineage != expected_lineage_digest || prefix_semantic != expected_semantic_digest {
        return Err("Codex rollout rewrote the expected prefix".to_string());
    }
    Ok(())
}

fn native_thread_from_path(
    codex_home: &Path,
    transcript_path: &Path,
) -> Result<NativeCodexThread, String> {
    native_thread_with_bytes_from_path(codex_home, transcript_path).map(|(thread, _)| thread)
}

fn native_thread_with_bytes_from_path(
    codex_home: &Path,
    transcript_path: &Path,
) -> Result<(NativeCodexThread, Vec<u8>), String> {
    let canonical_home =
        fs::canonicalize(codex_home).map_err(|error| format!("resolve Codex home: {error}"))?;
    if canonical_home != codex_home {
        return Err("Codex home is not canonical".to_string());
    }
    let canonical_path = fs::canonicalize(transcript_path)
        .map_err(|error| format!("resolve Codex rollout: {error}"))?;
    if canonical_path != transcript_path {
        return Err("Codex rollout path is not canonical".to_string());
    }
    if !is_rollout_jsonl(&canonical_path) {
        return Err("Codex rollout path is not a rollout JSONL file".to_string());
    }
    let metadata = fs::symlink_metadata(&canonical_path)
        .map_err(|error| format!("inspect Codex rollout: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Codex rollout is not a regular no-follow file".to_string());
    }
    if metadata.len() == 0 || metadata.len() > MAX_CONVERSATION_FILE_BYTES {
        return Err("Codex rollout is empty or exceeds its read limit".to_string());
    }

    let mut contained = false;
    for directory in ["sessions", "archived_sessions"] {
        let root = canonical_home.join(directory);
        if !canonical_path.starts_with(&root) {
            continue;
        }
        let root_metadata = fs::symlink_metadata(&root)
            .map_err(|error| format!("inspect Codex {directory}: {error}"))?;
        if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
            return Err(format!("Codex {directory} is not a real directory"));
        }
        let canonical_root = fs::canonicalize(&root)
            .map_err(|error| format!("resolve Codex {directory}: {error}"))?;
        if canonical_root != root || !canonical_path.starts_with(&canonical_root) {
            return Err("Codex rollout resolves outside its session tree".to_string());
        }
        contained = true;
        break;
    }
    if !contained {
        return Err("Codex rollout is outside the active and archived session trees".to_string());
    }

    let bytes = read_bounded_file(&canonical_path, MAX_CONVERSATION_FILE_BYTES)?;
    let thread = derive_native_thread(&canonical_path, &bytes)?
        .ok_or_else(|| "Codex rollout does not contain a complete native thread".to_string())?;
    Ok((thread, bytes))
}

/// Derive one exact rollout from bytes obtained by bounded verification.
fn derive_native_thread(
    canonical_path: &Path,
    bytes: &[u8],
) -> Result<Option<NativeCodexThread>, String> {
    let Ok(structure) = analyze_codex_rollout_structure(bytes, None) else {
        return Ok(None);
    };
    let analysis = analyze_codex_transcript(bytes);
    let (lineage_digest, semantic_digest) = match analysis.lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (lineage_digest, semantic_digest),
        CodecLineage::Unknown { .. } => return Ok(None),
    };
    let (history_record_digest, history_record_count) = non_metadata_record_digest(bytes)?;
    Ok(Some(NativeCodexThread {
        provider_thread_id: structure.current.id.clone(),
        transcript_path: canonical_path.to_path_buf(),
        transcript_sha256: hex_digest(Sha256::digest(bytes).as_slice()),
        cwd: PathBuf::from(&structure.current.cwd),
        forked_from_id: structure.current.forked_from_id.clone(),
        lineage_digest,
        semantic_digest,
        history_record_digest,
        history_record_count,
    }))
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
        validate_execution_artifacts_and_source(request)
    }) {
        return Ok(CodexForkCoordinatorOutcome::AbortedBeforeInvocation { message });
    }
    run_executor_sequence(
        &request.available_executors,
        record_attempt,
        |executor| match executor {
            CodexForkExecutor::AppServerStdio => try_app_server_fork(request, on_phase),
            CodexForkExecutor::NativePty => try_native_pty_fork(request, on_phase),
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
        validate_execution_artifacts_and_source(request)
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
            ExecutorOutcome::Unacknowledged(message) => {
                record_attempt(*executor, CodexForkAttemptStatus::Unacknowledged)?;
                return Ok(CodexForkCoordinatorOutcome::Unacknowledged {
                    executor: *executor,
                    message,
                });
            }
        }
    }
    Ok(CodexForkCoordinatorOutcome::AbortedBeforeInvocation {
        message: "Codex fork could not be invoked by an available executor".to_string(),
    })
}

pub fn verify_recorded_fork(
    request: &CodexForkExecutionRequest,
    provider_thread_id: &str,
    transcript_path: &Path,
    transcript_sha256: &str,
) -> Result<(), String> {
    validate_execution_artifacts_and_source(request)?;
    let candidate =
        native_thread_at_path(&request.codex_home, transcript_path, provider_thread_id)?;
    if candidate.transcript_sha256 != transcript_sha256 {
        return Err("verified Codex fork changed before local preservation".to_string());
    }
    let source = source_native_thread(request)?;
    verify_candidate_against_source(request, &candidate, &source)
}

pub fn verify_source_idle_and_title(request: &CodexForkExecutionRequest) -> Result<(), String> {
    validate_execution_artifacts_and_source(request)?;
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
    let original_name_status = match (&request.original_rename, transport_error.clone()) {
        (None, _) => CodexNameStatus::Unchanged,
        (Some(_), Some(error)) => CodexNameStatus::Failed(format!(
            "skipped after the preserved-thread naming request failed: {error}"
        )),
        (Some(rename), None) => match name_with_app_server_client(
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
        },
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
    let response = match timed(on_phase, "app-server: fork metadata only", || {
        client.fork_thread(
            &request.source_provider_thread_id,
            &request.project_directory,
        )
    }) {
        Ok(response) => response,
        Err(error) => {
            client.terminate_now();
            return Ok(ExecutorOutcome::Unacknowledged(error));
        }
    };
    if response.get("error").is_some() {
        client.terminate_now();
        return Ok(ExecutorOutcome::Unacknowledged(app_server_error(
            "thread/fork",
            &response,
        )));
    }
    let Some(candidate_id) = response
        .pointer("/result/thread/id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        client.terminate_now();
        return Ok(ExecutorOutcome::Unacknowledged(
            "Codex app-server fork response lacked a thread id".to_string(),
        ));
    };
    if let Some(parent) = response
        .pointer("/result/thread/forkedFromId")
        .and_then(Value::as_str)
    {
        if parent != request.source_provider_thread_id {
            client.terminate_now();
            return Ok(ExecutorOutcome::Unacknowledged(
                "Codex app-server returned a fork with the wrong parent".to_string(),
            ));
        }
    }

    let candidate_path = response
        .pointer("/result/thread/path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    client.terminate_now();
    let candidate = match candidate_path {
        Some(path) => match timed(on_phase, "app-server: verify returned path", || {
            verify_app_server_candidate_path(request, &candidate_id, &path)
        }) {
            Ok(candidate) => candidate,
            Err(error) => return Ok(ExecutorOutcome::Unacknowledged(error)),
        },
        None => {
            return Ok(ExecutorOutcome::Unacknowledged(
                "Codex app-server fork response lacked an exact rollout path".to_string(),
            ))
        }
    };
    Ok(ExecutorOutcome::Verified(VerifiedNativeFork {
        provider_thread_id: candidate.provider_thread_id,
        transcript_path: candidate.transcript_path,
        transcript_sha256: candidate.transcript_sha256,
        executor: CodexForkExecutor::AppServerStdio,
        name_status: CodexNameStatus::Deferred,
    }))
}

fn verify_app_server_candidate_path(
    request: &CodexForkExecutionRequest,
    candidate_id: &str,
    candidate_path: &Path,
) -> Result<NativeCodexThread, String> {
    let candidate = native_thread_at_path(&request.codex_home, candidate_path, candidate_id)?;
    let source = source_native_thread(request)?;
    if source.transcript_sha256 != request.source_transcript_sha256
        || source.lineage_digest != request.local_lineage_digest
        || source.semantic_digest != request.local_semantic_digest
    {
        return Err("Codex source thread changed during native fork".to_string());
    }
    verify_candidate_against_source(request, &candidate, &source)?;
    Ok(candidate)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ForkWatchSignal {
    Created(Vec<PathBuf>),
    Changed(Vec<PathBuf>),
    Lost,
}

struct ForkRolloutWatcher {
    watcher: RecommendedWatcher,
    receiver: Receiver<ForkWatchSignal>,
    sessions_root: PathBuf,
    watching_sessions: bool,
    lost: Arc<AtomicBool>,
}

impl ForkRolloutWatcher {
    fn start(codex_home: &Path) -> Result<Self, String> {
        validate_canonical_directory(codex_home, "Codex home")?;
        let sessions_root = codex_home.join("sessions");
        let watching_sessions = sessions_root.exists();
        let (watch_root, recursive_mode) = if watching_sessions {
            validate_canonical_directory(&sessions_root, "Codex sessions")?;
            (sessions_root.clone(), RecursiveMode::Recursive)
        } else {
            // Observe only the home directory entry until Codex creates its
            // sessions tree. Do not recursively watch the whole profile.
            (codex_home.to_path_buf(), RecursiveMode::NonRecursive)
        };
        let (sender, receiver) = mpsc::sync_channel(NATIVE_PTY_WATCH_QUEUE);
        let lost = Arc::new(AtomicBool::new(false));
        let callback_lost = Arc::clone(&lost);
        let callback_sessions_root = sessions_root.clone();
        let mut watcher = RecommendedWatcher::new(
            move |result| {
                let signal = match result {
                    Ok(event) => classify_fork_watch_event(event).and_then(|signal| {
                        scope_fork_watch_signal(&callback_sessions_root, signal)
                    }),
                    Err(_) => Some(ForkWatchSignal::Lost),
                };
                if let Some(signal) = signal {
                    send_fork_watch_signal(&sender, &callback_lost, signal);
                }
            },
            Config::default().with_follow_symlinks(false),
        )
        .map_err(|_| "Codex session watcher could not be created".to_string())?;
        watcher
            .watch(&watch_root, recursive_mode)
            .map_err(|_| "Codex session tree could not be watched".to_string())?;
        Ok(Self {
            watcher,
            receiver,
            sessions_root,
            watching_sessions,
            lost,
        })
    }

    fn watch_sessions_if_created(&mut self) -> Result<(), String> {
        if self.watching_sessions || !self.sessions_root.exists() {
            return Ok(());
        }
        validate_canonical_directory(&self.sessions_root, "Codex sessions")?;
        self.watcher
            .watch(&self.sessions_root, RecursiveMode::Recursive)
            .map_err(|_| "Codex session tree could not be watched".to_string())?;
        self.watching_sessions = true;
        Ok(())
    }
}

fn send_fork_watch_signal(
    sender: &SyncSender<ForkWatchSignal>,
    lost: &AtomicBool,
    signal: ForkWatchSignal,
) {
    match sender.try_send(signal) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => lost.store(true, Ordering::Release),
        Err(TrySendError::Disconnected(_)) => {}
    }
}

fn classify_fork_watch_event(event: Event) -> Option<ForkWatchSignal> {
    if event.need_rescan() || event.paths.len() > NATIVE_PTY_MAX_EVENT_PATHS {
        return Some(ForkWatchSignal::Lost);
    }
    match event.kind {
        EventKind::Create(_) => Some(ForkWatchSignal::Created(event.paths)),
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            Some(ForkWatchSignal::Created(event.paths))
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => event
            .paths
            .last()
            .cloned()
            .map(|path| ForkWatchSignal::Created(vec![path])),
        EventKind::Modify(ModifyKind::Name(_)) => None,
        EventKind::Modify(_) => Some(ForkWatchSignal::Changed(event.paths)),
        _ => None,
    }
}

fn scope_fork_watch_signal(
    sessions_root: &Path,
    signal: ForkWatchSignal,
) -> Option<ForkWatchSignal> {
    match signal {
        ForkWatchSignal::Created(paths) => {
            let paths = paths
                .into_iter()
                .filter(|path| {
                    path == sessions_root || is_watched_rollout_path(sessions_root, path)
                })
                .collect::<Vec<_>>();
            (!paths.is_empty()).then_some(ForkWatchSignal::Created(paths))
        }
        ForkWatchSignal::Changed(paths) => {
            let paths = paths
                .into_iter()
                .filter(|path| is_watched_rollout_path(sessions_root, path))
                .collect::<Vec<_>>();
            (!paths.is_empty()).then_some(ForkWatchSignal::Changed(paths))
        }
        ForkWatchSignal::Lost => Some(ForkWatchSignal::Lost),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativePtyShutdown {
    Observing,
    ExitSent(Instant),
    InterruptSent(Instant),
}

#[derive(Clone, Copy, Debug)]
struct NativePtyDrainReport {
    io_failed: bool,
}

#[derive(Clone, Copy, Debug)]
struct ObservedForkCandidate {
    dirty: bool,
    retry_at: Option<Instant>,
    retry_count: u8,
}

impl ObservedForkCandidate {
    fn created() -> Self {
        Self {
            dirty: true,
            retry_at: None,
            retry_count: 0,
        }
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.retry_at = None;
        self.retry_count = 0;
    }

    fn schedule_retry(&mut self, now: Instant) {
        if self.retry_count >= NATIVE_PTY_MAX_VERIFY_RETRIES {
            return;
        }
        let multiplier = 1_u32 << u32::from(self.retry_count.min(5));
        self.retry_at = Some(now + NATIVE_PTY_VERIFY_RETRY_BASE.saturating_mul(multiplier));
        self.retry_count = self.retry_count.saturating_add(1);
    }
}

fn observe_fork_candidates(
    request: &CodexForkExecutionRequest,
    watcher: &mut ForkRolloutWatcher,
    candidate_paths: &mut BTreeMap<PathBuf, ObservedForkCandidate>,
    matched_candidate: &mut Option<(PathBuf, String)>,
    stable_candidate: &mut Option<(VerifiedNativeFork, Instant)>,
    now: Instant,
) -> Result<Option<VerifiedNativeFork>, String> {
    collect_fork_watch_signals(watcher, candidate_paths)?;
    let stable_due_path = stable_candidate
        .as_ref()
        .filter(|(_, observed_at)| {
            now.saturating_duration_since(*observed_at)
                >= NATIVE_PTY_STABILITY_INTERVAL + NATIVE_PTY_COMPOSER_GRACE
        })
        .map(|(candidate, _)| candidate.transcript_path.clone());
    let paths_to_verify = candidate_paths
        .iter_mut()
        .filter_map(|(path, candidate)| {
            if candidate.dirty
                || candidate.retry_at.is_some_and(|retry_at| now >= retry_at)
                || stable_due_path.as_ref() == Some(path)
            {
                candidate.dirty = false;
                candidate.retry_at = None;
                Some(path.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let attempted_stable_path = stable_candidate
        .as_ref()
        .is_some_and(|(candidate, _)| paths_to_verify.contains(&candidate.transcript_path));
    let mut verified = Vec::new();
    for path in paths_to_verify {
        match verify_watched_candidate_path(request, &path) {
            Ok(candidate) => {
                if let Some(observed) = candidate_paths.get_mut(&path) {
                    observed.retry_count = 0;
                }
                verified.push(VerifiedNativeFork {
                    provider_thread_id: candidate.provider_thread_id,
                    transcript_path: candidate.transcript_path,
                    transcript_sha256: candidate.transcript_sha256,
                    executor: CodexForkExecutor::NativePty,
                    name_status: CodexNameStatus::Deferred,
                });
            }
            Err(_) => {
                if let Some(observed) = candidate_paths.get_mut(&path) {
                    observed.schedule_retry(now);
                }
            }
        }
    }
    if verified.len() > 1 {
        return Err(
            "multiple matching Codex rollout events arrived during the PTY fork".to_string(),
        );
    }
    if let Some(candidate) = verified.into_iter().next() {
        if matched_candidate.as_ref().is_some_and(|(path, thread_id)| {
            path != &candidate.transcript_path || thread_id != &candidate.provider_thread_id
        }) {
            return Err("the matching Codex rollout changed during PTY verification".to_string());
        }
        matched_candidate.get_or_insert_with(|| {
            (
                candidate.transcript_path.clone(),
                candidate.provider_thread_id.clone(),
            )
        });
        let observed_at = match stable_candidate.as_ref() {
            Some((existing, observed_at))
                if existing.provider_thread_id == candidate.provider_thread_id
                    && existing.transcript_path == candidate.transcript_path
                    && existing.transcript_sha256 == candidate.transcript_sha256 =>
            {
                *observed_at
            }
            _ => {
                *stable_candidate = Some((candidate.clone(), now));
                now
            }
        };
        if stable_due_path.as_ref() == Some(&candidate.transcript_path)
            && now.saturating_duration_since(observed_at)
                >= NATIVE_PTY_STABILITY_INTERVAL + NATIVE_PTY_COMPOSER_GRACE
        {
            return Ok(Some(candidate));
        }
    } else if attempted_stable_path {
        *stable_candidate = None;
    }
    Ok(None)
}

fn try_native_pty_fork(
    request: &CodexForkExecutionRequest,
    on_phase: ForkPhaseObserver<'_>,
) -> Result<ExecutorOutcome, String> {
    timed(on_phase, "pty: validate request", || {
        validate_execution_artifacts_and_source(request)
    })?;
    if !timed(on_phase, "pty: login status preflight", || {
        codex_login_status_ready(request)
    })? {
        return Ok(ExecutorOutcome::DefinitiveNoFork);
    }
    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: 40,
        cols: 120,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(pair) => pair,
        Err(_) => return Ok(ExecutorOutcome::DefinitiveNoFork),
    };
    let portable_pty::PtyPair { slave, master } = pair;
    let reader = match master.try_clone_reader() {
        Ok(reader) => reader,
        Err(_) => return Ok(ExecutorOutcome::DefinitiveNoFork),
    };
    let writer = match master.take_writer() {
        Ok(writer) => writer,
        Err(_) => return Ok(ExecutorOutcome::DefinitiveNoFork),
    };
    let watcher = match timed(on_phase, "pty: start rollout watcher", || {
        ForkRolloutWatcher::start(&request.codex_home)
    }) {
        Ok(watcher) => watcher,
        Err(_) => return Ok(ExecutorOutcome::DefinitiveNoFork),
    };
    let spawn_started = Instant::now();
    let command = build_native_pty_command(request);
    let child = match slave.spawn_command(command) {
        Ok(child) => child,
        Err(_) => return Ok(ExecutorOutcome::DefinitiveNoFork),
    };
    drop(slave);
    on_phase("pty: spawn codex fork", spawn_started.elapsed());
    let session_started = Instant::now();
    let session = run_native_pty_session(request, watcher, child, reader, writer, master);
    on_phase("pty: run session and observe", session_started.elapsed());
    match session {
        Ok(outcome) => Ok(outcome),
        Err(_) => Ok(ExecutorOutcome::Unacknowledged(
            "Mallard did not acknowledge the native PTY fork".to_string(),
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

fn run_native_pty_session(
    request: &CodexForkExecutionRequest,
    mut watcher: ForkRolloutWatcher,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    mut reader: Box<dyn Read + Send>,
    mut writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
) -> Result<ExecutorOutcome, String> {
    let (drain_sender, drain_receiver) = mpsc::channel();
    let drain_worker = thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        let io_failed = loop {
            match reader.read(&mut buffer) {
                Ok(0) => break false,
                Ok(_) => {}
                Err(_) => break true,
            }
        };
        let report = NativePtyDrainReport { io_failed };
        let _ = drain_sender.send(report);
        report
    });

    let operation_deadline = Instant::now() + NATIVE_PTY_TIMEOUT;
    let mut shutdown = NativePtyShutdown::Observing;
    let mut candidate_paths = BTreeMap::<PathBuf, ObservedForkCandidate>::new();
    let mut matched_candidate: Option<(PathBuf, String)> = None;
    let mut stable_candidate: Option<(VerifiedNativeFork, Instant)> = None;
    let mut acknowledged = None;
    let mut unacknowledged = None;
    let mut drain_report = None;

    loop {
        let now = Instant::now();
        if shutdown == NativePtyShutdown::Observing {
            match observe_fork_candidates(
                request,
                &mut watcher,
                &mut candidate_paths,
                &mut matched_candidate,
                &mut stable_candidate,
                now,
            ) {
                Ok(Some(candidate)) => {
                    acknowledged = Some(candidate);
                    if writer
                        .write_all(b"/exit\r")
                        .and_then(|_| writer.flush())
                        .is_err()
                    {
                        if !terminate_native_pty_child(&mut *child) {
                            unacknowledged = Some(
                                "native PTY cleanup was uncertain after fork verification"
                                    .to_string(),
                            );
                        }
                        break;
                    }
                    shutdown = NativePtyShutdown::ExitSent(now + NATIVE_PTY_EXIT_GRACE);
                }
                Ok(None) => {}
                Err(message) => {
                    unacknowledged = Some(message);
                    let _ = terminate_native_pty_child(&mut *child);
                    break;
                }
            }
        }

        match child.try_wait() {
            Ok(Some(_)) => {
                if child.wait().is_err() {
                    let terminated = terminate_native_pty_child(&mut *child);
                    if acknowledged.is_none() || !terminated {
                        unacknowledged = Some("native PTY child could not be reaped".to_string());
                    }
                } else if acknowledged.is_none() {
                    let final_deadline = Instant::now() + NATIVE_PTY_EXIT_OBSERVATION_GRACE;
                    loop {
                        let observed_at = Instant::now();
                        match observe_fork_candidates(
                            request,
                            &mut watcher,
                            &mut candidate_paths,
                            &mut matched_candidate,
                            &mut stable_candidate,
                            observed_at,
                        ) {
                            Ok(Some(candidate)) => {
                                acknowledged = Some(candidate);
                                break;
                            }
                            Ok(None) if observed_at < final_deadline => {
                                thread::sleep(NATIVE_PTY_EVENT_INTERVAL);
                            }
                            Ok(None) => {
                                unacknowledged = Some(
                                    "native PTY exited before Mallard acknowledged its fork"
                                        .to_string(),
                                );
                                break;
                            }
                            Err(message) => {
                                unacknowledged = Some(message);
                                break;
                            }
                        }
                    }
                }
                break;
            }
            Ok(None) => {}
            Err(_) => {
                let terminated = terminate_native_pty_child(&mut *child);
                if acknowledged.is_none() || !terminated {
                    unacknowledged = Some("native PTY child status became uncertain".to_string());
                }
                break;
            }
        }

        if drain_report.is_none() && drain_worker.is_finished() {
            drain_report = drain_receiver.try_recv().ok();
            let message = match drain_report {
                Some(_) => "native PTY output drain ended before Codex exited".to_string(),
                None => "native PTY output drain worker failed".to_string(),
            };
            let terminated = terminate_native_pty_child(&mut *child);
            if acknowledged.is_none() || !terminated {
                unacknowledged = Some(message);
            }
            break;
        }

        match shutdown {
            NativePtyShutdown::Observing => {
                if now >= operation_deadline {
                    unacknowledged =
                        Some("native PTY fork was not acknowledged before timeout".to_string());
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
                    if !terminate_native_pty_child(&mut *child) {
                        unacknowledged = Some(
                            "native PTY cleanup was uncertain after fork verification".to_string(),
                        );
                    }
                    break;
                }
                shutdown = NativePtyShutdown::InterruptSent(now + NATIVE_PTY_INTERRUPT_GRACE);
            }
            NativePtyShutdown::InterruptSent(deadline) if now >= deadline => {
                if !terminate_native_pty_child(&mut *child) && acknowledged.is_some() {
                    unacknowledged = Some(
                        "native PTY cleanup was uncertain after fork verification".to_string(),
                    );
                }
                break;
            }
            NativePtyShutdown::ExitSent(_) | NativePtyShutdown::InterruptSent(_) => {}
        }
        thread::sleep(NATIVE_PTY_EVENT_INTERVAL);
    }

    drop(writer);
    drop(master);
    if drain_report.is_none() {
        drain_report = drain_receiver.recv_timeout(NATIVE_PTY_DRAIN_GRACE).ok();
    }
    // Dropping a JoinHandle detaches it. Never let a failed child termination
    // turn the bounded fork attempt into an unbounded wait on a blocked PTY
    // read.
    drop(drain_worker);
    match drain_report {
        Some(report) if report.io_failed && acknowledged.is_none() => {
            unacknowledged = Some("native PTY output drain failed before cleanup".to_string());
        }
        None if acknowledged.is_none() => {
            unacknowledged =
                Some("native PTY output drain did not finish during cleanup".to_string());
        }
        Some(_) | None => {}
    }

    Ok(match (acknowledged, unacknowledged) {
        (Some(fork), None) => ExecutorOutcome::Verified(fork),
        (_, Some(message)) => ExecutorOutcome::Unacknowledged(message),
        (None, None) => ExecutorOutcome::Unacknowledged(
            "Mallard did not acknowledge the native PTY fork".to_string(),
        ),
    })
}

fn collect_fork_watch_signals(
    watcher: &mut ForkRolloutWatcher,
    candidates: &mut BTreeMap<PathBuf, ObservedForkCandidate>,
) -> Result<(), String> {
    watcher.watch_sessions_if_created()?;
    if watcher.lost.load(Ordering::Acquire) {
        return Err("Codex session watcher lost events".to_string());
    }
    loop {
        match watcher.receiver.try_recv() {
            Ok(ForkWatchSignal::Lost) => {
                return Err("Codex session watcher lost events".to_string())
            }
            Ok(ForkWatchSignal::Created(paths)) => {
                for path in paths {
                    if is_watched_rollout_path(&watcher.sessions_root, &path) {
                        candidates
                            .entry(path)
                            .or_insert_with(ObservedForkCandidate::created)
                            .changed();
                        if candidates.len() > NATIVE_PTY_MAX_CANDIDATES {
                            return Err("Codex session watcher saw too many candidates".to_string());
                        }
                    }
                }
            }
            Ok(ForkWatchSignal::Changed(paths)) => {
                // A change can complete a rollout first reported by Create or
                // rename-to. It never promotes an old file into a candidate.
                for path in paths {
                    if let Some(candidate) = candidates.get_mut(&path) {
                        candidate.changed();
                    }
                }
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                return Err("Codex session watcher disconnected".to_string())
            }
        }
    }
    if watcher.lost.load(Ordering::Acquire) {
        return Err("Codex session watcher lost events".to_string());
    }
    Ok(())
}

fn is_watched_rollout_path(sessions_root: &Path, path: &Path) -> bool {
    path.is_absolute() && path.starts_with(sessions_root) && is_rollout_jsonl(path)
}

fn verify_watched_candidate_path(
    request: &CodexForkExecutionRequest,
    candidate_path: &Path,
) -> Result<NativeCodexThread, String> {
    let candidate = native_thread_from_path(&request.codex_home, candidate_path)?;
    let source = source_native_thread(request)?;
    if source.transcript_sha256 != request.source_transcript_sha256
        || source.lineage_digest != request.local_lineage_digest
        || source.semantic_digest != request.local_semantic_digest
    {
        return Err("Codex source thread changed during native fork".to_string());
    }
    verify_candidate_against_source(request, &candidate, &source)?;
    Ok(candidate)
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
    if !request.source_transcript_path.is_absolute() {
        return Err("Codex source transcript path is not absolute".to_string());
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

fn source_native_thread(request: &CodexForkExecutionRequest) -> Result<NativeCodexThread, String> {
    native_thread_at_path(
        &request.codex_home,
        &request.source_transcript_path,
        &request.source_provider_thread_id,
    )
}

fn verify_source_unchanged(request: &CodexForkExecutionRequest) -> Result<(), String> {
    let source = source_native_thread(request)?;
    if source.transcript_sha256 != request.source_transcript_sha256
        || source.lineage_digest != request.local_lineage_digest
        || source.semantic_digest != request.local_semantic_digest
    {
        return Err("Codex source thread changed after Pull review".to_string());
    }
    Ok(())
}

fn verify_candidate_against_source(
    request: &CodexForkExecutionRequest,
    candidate: &NativeCodexThread,
    source: &NativeCodexThread,
) -> Result<(), String> {
    verify_candidate_workspace(request, candidate)?;
    if candidate.provider_thread_id == request.source_provider_thread_id
        || candidate.forked_from_id.as_deref() != Some(&request.source_provider_thread_id)
        || candidate.lineage_digest != request.local_lineage_digest
        || candidate.semantic_digest != request.local_semantic_digest
        || candidate.history_record_digest != source.history_record_digest
        || candidate.history_record_count != source.history_record_count
    {
        return Err("Codex fork candidate failed ancestry or content verification".to_string());
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
    if crate::codex_rollout::structural_record_type(value)
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
    let path_metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect bounded Codex file: {error}"))?;
    if !path_metadata.is_file()
        || path_metadata.file_type().is_symlink()
        || path_metadata.len() > limit
    {
        return Err("Codex file is not a bounded regular file".to_string());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("open bounded Codex file: {error}"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("inspect opened Codex file: {error}"))?;
    if !opened_metadata.is_file()
        || opened_metadata.len() > limit
        || !same_file_identity(&path_metadata, &opened_metadata)
    {
        return Err("Codex file is not a bounded regular file".to_string());
    }
    let post_open_metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("reinspect bounded Codex file: {error}"))?;
    if !post_open_metadata.is_file()
        || post_open_metadata.file_type().is_symlink()
        || !same_file_identity(&opened_metadata, &post_open_metadata)
    {
        return Err("Codex file changed during no-follow open".to_string());
    }
    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read bounded Codex file: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err("Codex file exceeds its read limit".to_string());
    }
    let final_opened_metadata = file
        .metadata()
        .map_err(|error| format!("reinspect opened Codex file: {error}"))?;
    let final_path_metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("reinspect bounded Codex path: {error}"))?;
    if !same_file_identity(&opened_metadata, &final_opened_metadata)
        || !same_file_identity(&final_opened_metadata, &final_path_metadata)
        || final_path_metadata.file_type().is_symlink()
        || final_opened_metadata.len() != bytes.len() as u64
    {
        return Err("Codex file changed during bounded read".to_string());
    }
    Ok(bytes)
}

#[cfg(unix)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    left.volume_serial_number()
        .zip(left.file_index())
        .zip(right.volume_serial_number().zip(right.file_index()))
        .is_some_and(|((left_volume, left_index), (right_volume, right_index))| {
            left_volume == right_volume && left_index == right_index
        })
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
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
    use notify::event::{CreateKind, DataChange, Flag};
    use serde_json::json;
    use std::ffi::OsString;
    use std::io::IsTerminal;

    fn test_rollout(thread_id: &str, parent: Option<&str>, cwd: &Path) -> Vec<u8> {
        let records = [
            json!({
            "timestamp": "2026-07-29T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": thread_id,
                "forked_from_id": parent,
                "cwd": cwd.to_str().unwrap(),
            },
            }),
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
        ];
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

    fn test_request(root: &Path, source_id: &str) -> (CodexForkExecutionRequest, PathBuf) {
        let home = root.join("codex");
        let project = root.join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&project).unwrap();
        let home = fs::canonicalize(home).unwrap();
        let project = fs::canonicalize(project).unwrap();
        let source_path = write_test_rollout(&home, source_id, None, &project);
        let executable = root.join("codex-test-bin");
        fs::write(&executable, b"not an executable").unwrap();
        let executable = fs::canonicalize(executable).unwrap();
        let source = native_thread_at_path(&home, &source_path, source_id).unwrap();
        (
            CodexForkExecutionRequest {
                codex_home: home.clone(),
                project_directory: project,
                codex_executable: executable.clone(),
                codex_executable_sha256: sha256_file(&executable, 1024).unwrap(),
                source_provider_thread_id: source_id.to_string(),
                source_transcript_path: source.transcript_path,
                source_transcript_sha256: source.transcript_sha256,
                local_lineage_digest: source.lineage_digest,
                local_semantic_digest: source.semantic_digest,
                pinned_local_title: None,
                desired_fork_name: "Question — local before Pull · 2026-07-29 18:42Z".to_string(),
                available_executors: vec![
                    CodexForkExecutor::AppServerStdio,
                    CodexForkExecutor::NativePty,
                ],
            },
            home,
        )
    }

    #[test]
    fn planned_name_is_normalized_bounded_and_stable() {
        let name = planned_fork_name(
            &format!("  {}   graph  ", "界".repeat(200)),
            1_775_000_000,
            ForkOrigin::PullPreserve,
        )
        .unwrap();
        assert_eq!(name.chars().count(), FORK_NAME_LIMIT);
        assert!(name.contains("… — local before Pull · "));
        assert!(name.ends_with("Z"));
        assert!(!name.contains("  "));
        assert_eq!(
            name,
            planned_fork_name(
                &format!("  {}   graph  ", "界".repeat(200)),
                1_775_000_000,
                ForkOrigin::PullPreserve,
            )
            .unwrap()
        );
    }

    #[test]
    fn planned_name_marks_manual_forks_without_the_pull_phrase() {
        let name = planned_fork_name("Analyze graph", 1_775_000_000, ForkOrigin::Manual).unwrap();
        assert!(name.contains(" — fork · "), "{name}");
        assert!(!name.contains("local before Pull"), "{name}");
        assert!(name.ends_with("Z"));
    }

    #[test]
    fn planned_name_rejects_control_characters() {
        let error =
            planned_fork_name("Unsafe\nname", 1_775_000_000, ForkOrigin::PullPreserve).unwrap_err();
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
        let (mut request, _) = test_request(temp.path(), source_id);
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
    fn app_server_fork_params_request_metadata_only_with_the_reviewed_project_cwd() {
        let project = Path::new("/tmp/project with spaces/知识图");
        assert_eq!(
            app_server_fork_params("thread-a", project).unwrap(),
            json!({
                "threadId": "thread-a",
                "cwd": "/tmp/project with spaces/知识图",
                "excludeTurns": true,
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
    fn app_server_verifies_the_returned_rollout_path_without_scanning_the_profile() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let (mut request, home) = test_request(temp.path(), source_id);
        let child_path = home
            .join("sessions/2026/07/29")
            .join(format!("rollout-{child_id}.jsonl"));
        fs::write(
            home.join("fork-template.jsonl"),
            test_rollout(child_id, Some(source_id), &request.project_directory),
        )
        .unwrap();
        // A whole-profile scan would reject this tree. The response-path path
        // verifies only the pinned source and the returned child.
        fs::write(home.join("archived_sessions"), b"must not be scanned").unwrap();
        let script = r#"#!/bin/sh
while IFS= read -r line
do
  case "$line" in
    *'"method":"initialize"'*)
      case "$line" in
        *'"experimentalApi":true'*)
          printf '%s\n' '{"id":1,"result":{}}'
          ;;
      esac
      ;;
    *'"method":"thread/read"'*)
      printf '%s\n' '{"id":2,"result":{"thread":{"id":"__SOURCE_ID__","name":null,"status":{"type":"idle"}}}}'
      ;;
    *'"method":"thread/fork"'*)
      case "$line" in
        *'"excludeTurns":true'*)
          cp "$CODEX_HOME/fork-template.jsonl" "__CHILD_PATH__"
          printf '%s\n' '{"id":3,"result":{"thread":{"id":"__CHILD_ID__","forkedFromId":"__SOURCE_ID__","path":"__CHILD_PATH__","turns":[]}}}'
          ;;
      esac
      ;;
  esac
done
"#
        .replace("__SOURCE_ID__", source_id)
        .replace("__CHILD_ID__", child_id)
        .replace("__CHILD_PATH__", child_path.to_str().unwrap());
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
        assert_eq!(verified.transcript_path, child_path);
        assert_eq!(verified.executor, CodexForkExecutor::AppServerStdio);
        assert_eq!(verified.name_status, CodexNameStatus::Deferred);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "fork did not use the returned rollout path"
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
    fn bounded_file_identity_distinguishes_a_replacement_inode() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("original");
        let replacement = temp.path().join("replacement");
        fs::write(&original, b"same size").unwrap();
        fs::write(&replacement, b"same size").unwrap();
        let original_metadata = fs::metadata(&original).unwrap();
        let same_metadata = fs::metadata(&original).unwrap();
        let replacement_metadata = fs::metadata(&replacement).unwrap();

        assert!(same_file_identity(&original_metadata, &same_metadata));
        assert!(!same_file_identity(
            &original_metadata,
            &replacement_metadata
        ));
    }

    #[test]
    fn direct_rollout_lookup_rejects_a_matching_thread_outside_the_provider_trees() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let (request, _) = test_request(temp.path(), source_id);
        let outside = temp.path().join(format!("rollout-{child_id}.jsonl"));
        fs::write(
            &outside,
            test_rollout(child_id, Some(source_id), &request.project_directory),
        )
        .unwrap();
        let outside = fs::canonicalize(outside).unwrap();

        let error = native_thread_at_path(&request.codex_home, &outside, child_id).unwrap_err();
        assert!(error.contains("outside"), "{error}");
    }

    #[test]
    fn watch_event_classifier_promotes_create_and_rename_to() {
        let created_path = PathBuf::from("/tmp/rollout-created.jsonl");
        let created =
            Event::new(EventKind::Create(CreateKind::File)).add_path(created_path.clone());
        assert_eq!(
            classify_fork_watch_event(created),
            Some(ForkWatchSignal::Created(vec![created_path]))
        );

        let renamed_path = PathBuf::from("/tmp/rollout-renamed.jsonl");
        let renamed = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::To)))
            .add_path(renamed_path.clone());
        assert_eq!(
            classify_fork_watch_event(renamed),
            Some(ForkWatchSignal::Created(vec![renamed_path]))
        );
    }

    #[test]
    fn watch_event_classifier_does_not_promote_modify_and_marks_rescan_lost() {
        let modified_path = PathBuf::from("/tmp/rollout-existing.jsonl");
        let modified = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
            .add_path(modified_path.clone());
        let modified_signal = classify_fork_watch_event(modified);
        assert_eq!(
            modified_signal,
            Some(ForkWatchSignal::Changed(vec![modified_path]))
        );

        let (sender, receiver) = mpsc::sync_channel(1);
        let backend =
            RecommendedWatcher::new(|_result: notify::Result<Event>| {}, Config::default())
                .unwrap();
        let mut watcher = ForkRolloutWatcher {
            watcher: backend,
            receiver,
            sessions_root: PathBuf::from("/tmp"),
            watching_sessions: true,
            lost: Arc::new(AtomicBool::new(false)),
        };
        sender.send(modified_signal.unwrap()).unwrap();
        let mut candidates = BTreeMap::new();
        collect_fork_watch_signals(&mut watcher, &mut candidates).unwrap();
        assert!(candidates.is_empty());

        let rescan = Event::new(EventKind::Any).set_flag(Flag::Rescan);
        let rescan_signal = classify_fork_watch_event(rescan);
        assert_eq!(rescan_signal, Some(ForkWatchSignal::Lost));
        sender.send(rescan_signal.unwrap()).unwrap();
        let error = collect_fork_watch_signals(&mut watcher, &mut candidates).unwrap_err();
        assert!(error.contains("lost events"), "{error}");
    }

    #[test]
    fn exact_watched_candidate_verification_ignores_unrelated_rollouts() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let unrelated_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644d";
        let (request, home) = test_request(temp.path(), source_id);
        let directory = home.join("sessions/2026/07/29");
        fs::write(directory.join("rollout-malformed.jsonl"), b"{").unwrap();
        write_test_rollout(
            &home,
            unrelated_id,
            Some(source_id),
            &request.project_directory,
        );
        fs::write(home.join("archived_sessions"), b"must not be scanned").unwrap();
        let child_path =
            write_test_rollout(&home, child_id, Some(source_id), &request.project_directory);

        let candidate = verify_watched_candidate_path(&request, &child_path).unwrap();

        assert_eq!(candidate.provider_thread_id, child_id);
        assert_eq!(candidate.transcript_path, child_path);
    }

    #[test]
    fn observed_create_is_retried_without_a_modify_event_until_exact_verification_is_stable() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let (request, home) = test_request(temp.path(), source_id);
        let child_path = home
            .join("sessions/2026/07/29")
            .join(format!("rollout-{child_id}.jsonl"));
        fs::write(&child_path, b"{").unwrap();

        let (sender, receiver) = mpsc::sync_channel(1);
        let backend =
            RecommendedWatcher::new(|_result: notify::Result<Event>| {}, Config::default())
                .unwrap();
        let mut watcher = ForkRolloutWatcher {
            watcher: backend,
            receiver,
            sessions_root: home.join("sessions"),
            watching_sessions: true,
            lost: Arc::new(AtomicBool::new(false)),
        };
        sender
            .send(ForkWatchSignal::Created(vec![child_path.clone()]))
            .unwrap();
        let mut candidates = BTreeMap::new();
        let mut matched = None;
        let mut stable = None;
        let first_observation = Instant::now();
        assert!(observe_fork_candidates(
            &request,
            &mut watcher,
            &mut candidates,
            &mut matched,
            &mut stable,
            first_observation,
        )
        .unwrap()
        .is_none());

        fs::write(
            &child_path,
            test_rollout(child_id, Some(source_id), &request.project_directory),
        )
        .unwrap();
        let parsed_at = first_observation + NATIVE_PTY_VERIFY_RETRY_BASE;
        assert!(observe_fork_candidates(
            &request,
            &mut watcher,
            &mut candidates,
            &mut matched,
            &mut stable,
            parsed_at,
        )
        .unwrap()
        .is_none());
        let verified = observe_fork_candidates(
            &request,
            &mut watcher,
            &mut candidates,
            &mut matched,
            &mut stable,
            parsed_at + NATIVE_PTY_STABILITY_INTERVAL + NATIVE_PTY_COMPOSER_GRACE,
        )
        .unwrap()
        .unwrap();
        assert_eq!(verified.provider_thread_id, child_id);
        assert_eq!(verified.transcript_path, child_path);
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
        let (mut request, home) = test_request(temp.path(), source_id);
        let executable = std::env::current_exe().unwrap();
        request.codex_executable = executable.clone();
        request.codex_executable_sha256 = sha256_file(&executable, 512 * 1024 * 1024).unwrap();
        let mut watcher = ForkRolloutWatcher::start(&request.codex_home).unwrap();
        // Keep the production watcher alive, but inject the exact create signal
        // deterministically so this unit test does not depend on host event
        // delivery. The worker probes only the expected path.
        let (test_event_sender, test_event_receiver) = mpsc::sync_channel(1);
        let test_event_guard = test_event_sender.clone();
        watcher.receiver = test_event_receiver;
        let observed_child_path = home
            .join("sessions/2026/07/29")
            .join(format!("rollout-{child_id}.jsonl"));
        let event_worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline && !observed_child_path.exists() {
                thread::sleep(Duration::from_millis(10));
            }
            assert!(observed_child_path.exists());
            test_event_sender
                .send(ForkWatchSignal::Created(vec![observed_child_path]))
                .unwrap();
        });
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
        command.arg("codex_native_fork::tests::fake_codex_pty_subprocess_entrypoint");
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
            run_native_pty_session(&request, watcher, child, reader, writer, master).unwrap();
        drop(test_event_guard);
        event_worker.join().unwrap();
        let debug = format!("{outcome:?}");
        let ExecutorOutcome::Verified(verified) = outcome else {
            panic!("fake Codex PTY did not produce an exact verified child: {debug}");
        };
        assert_eq!(verified.provider_thread_id, child_id);
        assert_eq!(verified.executor, CodexForkExecutor::NativePty);
        assert!(!debug.contains("MALLARD-PTY-OUTPUT-MUST-NOT-BE-RETAINED"));
    }

    #[derive(Debug)]
    struct UnkillableTestChild;

    impl portable_pty::ChildKiller for UnkillableTestChild {
        fn kill(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("injected kill failure"))
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(Self)
        }
    }

    impl portable_pty::Child for UnkillableTestChild {
        fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
            Ok(None)
        }

        fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
            Err(std::io::Error::other("injected wait failure"))
        }

        fn process_id(&self) -> Option<u32> {
            None
        }

        #[cfg(windows)]
        fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
            None
        }
    }

    struct DelayedEofReader;

    impl Read for DelayedEofReader {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            thread::sleep(Duration::from_secs(2));
            Ok(0)
        }
    }

    struct FailingTestWriter;

    impl Write for FailingTestWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn verified_candidate_with_uncertain_pty_cleanup_is_unacknowledged() {
        let temp = tempfile::tempdir().unwrap();
        let source_id = "019fac21-4aba-7fc2-bb7d-df711c950987";
        let child_id = "019fac73-c1ff-79b3-bb8e-1673ed0d644c";
        let (request, home) = test_request(temp.path(), source_id);
        let child_path =
            write_test_rollout(&home, child_id, Some(source_id), &request.project_directory);
        let mut watcher = ForkRolloutWatcher::start(&request.codex_home).unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        watcher.receiver = receiver;
        sender
            .send(ForkWatchSignal::Created(vec![child_path]))
            .unwrap();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let portable_pty::PtyPair { slave, master } = pair;
        drop(slave);

        let outcome = run_native_pty_session(
            &request,
            watcher,
            Box::new(UnkillableTestChild),
            Box::new(DelayedEofReader),
            Box::new(FailingTestWriter),
            master,
        )
        .unwrap();

        assert!(matches!(
            outcome,
            ExecutorOutcome::Unacknowledged(message)
                if message.contains("cleanup was uncertain")
        ));
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
    fn executor_sequence_falls_through_to_pty_only_after_definitive_no_fork() {
        let mut invoked = Vec::new();
        let outcome = run_executor_sequence(
            &[
                CodexForkExecutor::AppServerStdio,
                CodexForkExecutor::NativePty,
            ],
            |_, _| Ok(()),
            |executor| {
                invoked.push(executor);
                Ok(match executor {
                    CodexForkExecutor::AppServerStdio => ExecutorOutcome::DefinitiveNoFork,
                    CodexForkExecutor::NativePty => {
                        ExecutorOutcome::Verified(test_verified_fork(executor))
                    }
                })
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
        assert!(matches!(outcome, CodexForkCoordinatorOutcome::Verified(_)));
    }

    #[test]
    fn executor_unacknowledged_stops_fallthrough() {
        let mut invoked = Vec::new();
        let mut attempts = Vec::new();
        let outcome = run_executor_sequence(
            &[
                CodexForkExecutor::AppServerStdio,
                CodexForkExecutor::NativePty,
                CodexForkExecutor::AppServerStdio,
            ],
            |executor, status| {
                attempts.push((executor, status));
                Ok(())
            },
            |executor| {
                invoked.push(executor);
                Ok(match executor {
                    CodexForkExecutor::AppServerStdio => ExecutorOutcome::DefinitiveNoFork,
                    CodexForkExecutor::NativePty => {
                        ExecutorOutcome::Unacknowledged("not observed".to_string())
                    }
                })
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
                (
                    CodexForkExecutor::NativePty,
                    CodexForkAttemptStatus::Pending,
                ),
                (
                    CodexForkExecutor::NativePty,
                    CodexForkAttemptStatus::Unacknowledged,
                ),
            ]
        );
        assert_eq!(
            outcome,
            CodexForkCoordinatorOutcome::Unacknowledged {
                executor: CodexForkExecutor::NativePty,
                message: "not observed".to_string(),
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
