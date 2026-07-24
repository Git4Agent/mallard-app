//! Provider-native conversation forking for Codex and Claude Code.
//!
//! A Mallard fork mints a new cloud thread; this module gives that thread a
//! provider-native session. The provider CLI is invoked first so the
//! provider itself owns identity, lineage, and indexing wherever it can
//! (`claude --fork-session`, `codex fork`); every invocation is verified by
//! post-conditions on the provider home rather than trusted. For Codex a
//! codec fallback copies the rollout directly and then reconstructs the
//! `state_<N>.sqlite` thread index so the forked thread stays visible to
//! `codex resume`. Claude has no fallback: its transcript format is
//! documented as internal, so only the CLI may write it.
//!
//! Invocations run through an injected [`ProviderCliRunner`] so tests use
//! stub binaries inside temporary homes and never touch `~/.codex` or
//! `~/.claude`.

use chrono::{DateTime, Utc};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::conversation_codec::{analyze_codex_transcript, rewrite_codex_transcript};
use super::domain::classify_conversation_relation;
use super::domain::ConversationRelation;

const MAX_CLI_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_TRANSCRIPT_BYTES: u64 = 512 * 1024 * 1024;
/// The fork itself happens at session load; the prompt exists only because
/// `claude -p` requires one. It is deliberately inert.
const CLAUDE_FORK_PROMPT: &str =
    "This session was forked by Mallard for sync. Reply with exactly: ok";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCliInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub envs: Vec<(String, String)>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCliOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs one provider CLI invocation. Injected so integration tests use stub
/// executables in temporary homes and unit tests use pure fakes.
pub trait ProviderCliRunner {
    fn run(&self, invocation: &ProviderCliInvocation) -> Result<ProviderCliOutput, String>;
}

/// Production runner: bounded output capture, hard timeout, no stdin.
pub struct SystemProviderCliRunner;

impl ProviderCliRunner for SystemProviderCliRunner {
    fn run(&self, invocation: &ProviderCliInvocation) -> Result<ProviderCliOutput, String> {
        let mut command = Command::new(&invocation.program);
        command
            .args(&invocation.args)
            .current_dir(&invocation.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &invocation.envs {
            command.env(key, value);
        }
        let mut child = command.spawn().map_err(|error| {
            format!(
                "spawn provider CLI '{}': {}",
                invocation.program.display(),
                error
            )
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "provider CLI stdout unavailable".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "provider CLI stderr unavailable".to_string())?;
        let stdout_reader = std::thread::spawn(move || read_capped(stdout));
        let stderr_reader = std::thread::spawn(move || read_capped(stderr));
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if started.elapsed() > invocation.timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = stdout_reader.join();
                        let _ = stderr_reader.join();
                        return Err(format!(
                            "provider CLI '{}' timed out after {:?}",
                            invocation.program.display(),
                            invocation.timeout
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(format!("wait for provider CLI: {}", error)),
            }
        };
        let stdout = stdout_reader
            .join()
            .map_err(|_| "provider CLI stdout reader panicked".to_string())?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| "provider CLI stderr reader panicked".to_string())?;
        Ok(ProviderCliOutput {
            success: status.success(),
            stdout,
            stderr,
        })
    }
}

fn read_capped(mut source: impl Read) -> String {
    let mut buffer = Vec::new();
    let _ = source
        .by_ref()
        .take(MAX_CLI_OUTPUT_BYTES as u64)
        .read_to_end(&mut buffer);
    // Drain the rest so the child never blocks on a full pipe.
    let mut sink = [0_u8; 8192];
    while matches!(source.read(&mut sink), Ok(count) if count > 0) {}
    String::from_utf8_lossy(&buffer).into_owned()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodexIndexOutcome {
    /// The thread row was inserted into the newest `state_<N>.sqlite`.
    Inserted,
    /// A row for this thread already existed.
    AlreadyIndexed,
    /// Reconstruction was not possible; the reason is diagnostic only.
    /// Codex's own backfill rebuilds the index from rollout files, so a
    /// skipped reconstruction degrades discoverability, never correctness.
    Skipped { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderForkMethod {
    ClaudeCli,
    CodexCli,
    CodexCodec { index: CodexIndexOutcome },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkedProviderSession {
    pub provider_thread_id: String,
    pub forked_from_provider_thread_id: String,
    pub transcript_path: PathBuf,
    pub method: ProviderForkMethod,
    /// True when a previous attempt already produced this fork; the call
    /// verified and reused it instead of forking again.
    pub already_existed: bool,
}

/// Claude Code stores transcripts under
/// `<config>/projects/<sanitized-cwd>/<session-id>.jsonl`, where the cwd is
/// sanitized by replacing every non-alphanumeric character with `-`.
pub fn claude_project_dir_name(project_root: &Path) -> String {
    project_root
        .to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn valid_uuid_v4(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
            }
        })
}

fn valid_provider_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

#[derive(Clone, Debug)]
pub struct ClaudeForkRequest {
    pub claude_home: PathBuf,
    pub project_root: PathBuf,
    pub source_session_id: String,
    /// Minted at plan time with [`super::domain::generated_uuid_v4`]. Claude
    /// requires a UUID and refuses an existing one, which makes retries
    /// detectable: if the target transcript exists the fork already happened.
    pub new_session_id: String,
    pub binary: PathBuf,
    pub timeout: Duration,
}

/// Fork a Claude Code session with the provider CLI:
/// `claude --resume <src> --fork-session --session-id <new> -p …`.
/// The CLI is the only writer of Claude transcripts — their format is
/// documented as internal — so there is no codec fallback.
pub fn fork_claude_session(
    runner: &dyn ProviderCliRunner,
    request: &ClaudeForkRequest,
) -> Result<ForkedProviderSession, String> {
    if !valid_provider_session_id(&request.source_session_id) {
        return Err("claude fork source session id is invalid".to_string());
    }
    if !valid_uuid_v4(&request.new_session_id) {
        return Err("claude fork requires a lowercase UUIDv4 session id".to_string());
    }
    let project_dir = request
        .claude_home
        .join("projects")
        .join(claude_project_dir_name(&request.project_root));
    let target = project_dir.join(format!("{}.jsonl", request.new_session_id));
    if transcript_present(&target)? {
        return Ok(ForkedProviderSession {
            provider_thread_id: request.new_session_id.clone(),
            forked_from_provider_thread_id: request.source_session_id.clone(),
            transcript_path: target,
            method: ProviderForkMethod::ClaudeCli,
            already_existed: true,
        });
    }
    let invocation = ProviderCliInvocation {
        program: request.binary.clone(),
        args: vec![
            "--resume".to_string(),
            request.source_session_id.clone(),
            "--fork-session".to_string(),
            "--session-id".to_string(),
            request.new_session_id.clone(),
            "-p".to_string(),
            CLAUDE_FORK_PROMPT.to_string(),
        ],
        cwd: request.project_root.clone(),
        envs: vec![(
            "CLAUDE_CONFIG_DIR".to_string(),
            request.claude_home.to_string_lossy().into_owned(),
        )],
        timeout: request.timeout,
    };
    let output = runner.run(&invocation)?;
    if !transcript_present(&target)? {
        return Err(format!(
            "claude fork did not produce '{}' (exit success: {}): {}",
            target.display(),
            output.success,
            truncate_for_error(&output.stderr)
        ));
    }
    Ok(ForkedProviderSession {
        provider_thread_id: request.new_session_id.clone(),
        forked_from_provider_thread_id: request.source_session_id.clone(),
        transcript_path: target,
        method: ProviderForkMethod::ClaudeCli,
        already_existed: false,
    })
}

#[derive(Clone, Debug)]
pub struct CodexForkRequest {
    pub codex_home: PathBuf,
    pub project_root: PathBuf,
    pub source_thread_id: String,
    /// Minted at plan time; used verbatim by the codec fallback and as the
    /// idempotency key for retries. The CLI path lets Codex mint its own ID
    /// instead — Codex has no flag to choose one.
    pub new_thread_id: String,
    pub source_transcript: Vec<u8>,
    pub binary: PathBuf,
    pub timeout: Duration,
    /// Skip the CLI attempt entirely (missing binary, tests, or policy).
    pub allow_cli: bool,
    pub now: DateTime<Utc>,
}

/// Fork a Codex thread. CLI first (`codex fork <id>` with `CODEX_HOME`),
/// verified by discovering the new rollout and proving chain equality with
/// the source; on any CLI failure fall back to the codec copy plus SQLite
/// index reconstruction.
pub fn fork_codex_session(
    runner: &dyn ProviderCliRunner,
    request: &CodexForkRequest,
) -> Result<ForkedProviderSession, String> {
    if !valid_provider_session_id(&request.source_thread_id) {
        return Err("codex fork source thread id is invalid".to_string());
    }
    if !valid_uuid_v4(&request.new_thread_id) {
        return Err("codex fork requires a lowercase UUIDv4 thread id".to_string());
    }
    if request.source_transcript.len() as u64 > MAX_TRANSCRIPT_BYTES {
        return Err("codex fork source transcript exceeds the size limit".to_string());
    }
    let sessions_root = request.codex_home.join("sessions");

    // Retry detection for the codec path: the pinned ID travels in the plan,
    // so an existing rollout for it means a previous attempt completed.
    if let Some(existing) = find_rollout_by_thread_id(&sessions_root, &request.new_thread_id)? {
        let index = reconstruct_codex_sqlite_index(
            &request.codex_home,
            &request.new_thread_id,
            Some(&request.source_thread_id),
            &existing,
            request.now.timestamp(),
        );
        return Ok(ForkedProviderSession {
            provider_thread_id: request.new_thread_id.clone(),
            forked_from_provider_thread_id: request.source_thread_id.clone(),
            transcript_path: existing,
            method: ProviderForkMethod::CodexCodec { index },
            already_existed: true,
        });
    }

    if request.allow_cli {
        match fork_codex_via_cli(runner, request, &sessions_root) {
            Ok(forked) => return Ok(forked),
            Err(_reason) => {
                // The CLI is an enhancement, not the contract: fall through
                // to the codec copy, which is fully verifiable.
            }
        }
    }
    fork_codex_via_codec(request, &sessions_root)
}

fn fork_codex_via_cli(
    runner: &dyn ProviderCliRunner,
    request: &CodexForkRequest,
    sessions_root: &Path,
) -> Result<ForkedProviderSession, String> {
    let before = list_rollout_files(sessions_root)?;
    let invocation = ProviderCliInvocation {
        program: request.binary.clone(),
        args: vec!["fork".to_string(), request.source_thread_id.clone()],
        cwd: request.project_root.clone(),
        envs: vec![(
            "CODEX_HOME".to_string(),
            request.codex_home.to_string_lossy().into_owned(),
        )],
        timeout: request.timeout,
    };
    let output = runner.run(&invocation)?;
    if !output.success {
        return Err(format!(
            "codex fork exited unsuccessfully: {}",
            truncate_for_error(&output.stderr)
        ));
    }
    let after = list_rollout_files(sessions_root)?;
    let mut created = after
        .into_iter()
        .filter(|path| !before.contains(path))
        .collect::<Vec<_>>();
    if created.len() != 1 {
        return Err(format!(
            "codex fork produced {} new rollout files (expected exactly one)",
            created.len()
        ));
    }
    let rollout_path = created.remove(0);
    let bytes = read_bounded_transcript(&rollout_path)?;
    let analysis = analyze_codex_transcript(&bytes);
    let forked_thread_id = analysis
        .source_thread_id
        .clone()
        .ok_or_else(|| "codex fork rollout lacks a session id".to_string())?;
    if forked_thread_id == request.source_thread_id {
        return Err("codex fork rollout kept the source thread id".to_string());
    }
    let source_analysis = analyze_codex_transcript(&request.source_transcript);
    if classify_conversation_relation(
        analysis.lineage.chain(),
        source_analysis.lineage.chain(),
    ) != ConversationRelation::Identical
    {
        return Err("codex fork rollout does not match the source chain".to_string());
    }
    Ok(ForkedProviderSession {
        provider_thread_id: forked_thread_id,
        forked_from_provider_thread_id: request.source_thread_id.clone(),
        transcript_path: rollout_path,
        method: ProviderForkMethod::CodexCli,
        already_existed: false,
    })
}

fn fork_codex_via_codec(
    request: &CodexForkRequest,
    sessions_root: &Path,
) -> Result<ForkedProviderSession, String> {
    let project_root = request
        .project_root
        .to_str()
        .ok_or_else(|| "codex fork project root is not valid UTF-8".to_string())?;
    let rewritten = rewrite_codex_transcript(
        &request.source_transcript,
        &request.source_thread_id,
        &request.new_thread_id,
        project_root,
        project_root,
    )?;
    let day_dir = sessions_root
        .join(request.now.format("%Y").to_string())
        .join(request.now.format("%m").to_string())
        .join(request.now.format("%d").to_string());
    fs::create_dir_all(&day_dir)
        .map_err(|error| format!("create '{}': {}", day_dir.display(), error))?;
    let rollout_path = day_dir.join(format!(
        "rollout-{}-{}.jsonl",
        request.now.format("%Y-%m-%dT%H-%M-%S"),
        request.new_thread_id
    ));
    write_new_private_file(&rollout_path, &rewritten)?;
    let index = reconstruct_codex_sqlite_index(
        &request.codex_home,
        &request.new_thread_id,
        Some(&request.source_thread_id),
        &rollout_path,
        request.now.timestamp(),
    );
    Ok(ForkedProviderSession {
        provider_thread_id: request.new_thread_id.clone(),
        forked_from_provider_thread_id: request.source_thread_id.clone(),
        transcript_path: rollout_path,
        method: ProviderForkMethod::CodexCodec { index },
        already_existed: false,
    })
}

/// Rewrite a Claude transcript for materialization on another host: every
/// string field exactly equal to the source session ID becomes the
/// destination ID, and every field exactly equal to the source cwd becomes
/// the destination cwd. Claude's entry format is internal, so the rewrite is
/// deliberately schema-blind: lines are parsed as JSON, only exact string
/// values are replaced, and untouched lines are preserved byte-for-byte.
/// Placement of the result under `projects/<sanitized-cwd>/<session>.jsonl`
/// is Claude's whole index — there is no database to reconstruct.
pub fn rewrite_claude_transcript(
    bytes: &[u8],
    source_session_id: &str,
    destination_session_id: &str,
    source_cwd: &str,
    destination_cwd: &str,
) -> Result<Vec<u8>, String> {
    if !valid_uuid_v4(destination_session_id) {
        return Err("claude rewrite requires a lowercase UUIDv4 destination".to_string());
    }
    if !valid_provider_session_id(source_session_id) {
        return Err("claude rewrite source session id is invalid".to_string());
    }
    if bytes.len() as u64 > MAX_TRANSCRIPT_BYTES {
        return Err("claude transcript exceeds the size limit".to_string());
    }
    let mut output = Vec::with_capacity(bytes.len());
    let mut saw_destination = false;
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let needs_rewrite = contains_subslice(line, source_session_id.as_bytes())
            || contains_subslice(line, source_cwd.as_bytes());
        if !needs_rewrite {
            serde_json::from_slice::<serde_json::Value>(line)
                .map_err(|_| "claude transcript contains malformed JSON".to_string())?;
            output.extend_from_slice(line);
            output.push(b'\n');
            continue;
        }
        let mut value: serde_json::Value = serde_json::from_slice(line)
            .map_err(|_| "claude transcript contains malformed JSON".to_string())?;
        replace_exact_strings(&mut value, source_session_id, destination_session_id);
        replace_exact_strings(&mut value, source_cwd, destination_cwd);
        let rewritten = serde_json::to_vec(&value)
            .map_err(|error| format!("serialize rewritten claude row: {error}"))?;
        saw_destination = saw_destination
            || contains_subslice(&rewritten, destination_session_id.as_bytes());
        output.extend_from_slice(&rewritten);
        output.push(b'\n');
    }
    if !saw_destination {
        return Err(
            "claude rewrite found no session-id field to rebind; refusing to materialize"
                .to_string(),
        );
    }
    Ok(output)
}

fn replace_exact_strings(value: &mut serde_json::Value, from: &str, to: &str) {
    match value {
        serde_json::Value::String(text) => {
            if text == from {
                *text = to.to_string();
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                replace_exact_strings(item, from, to);
            }
        }
        serde_json::Value::Object(fields) => {
            for (_, field) in fields.iter_mut() {
                replace_exact_strings(field, from, to);
            }
        }
        _ => {}
    }
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
}

/// Insert the forked thread into the newest `state_<N>.sqlite` thread index
/// so it is discoverable by `codex resume` without waiting for Codex's own
/// backfill. The schema is Codex-internal and version-dependent, so every
/// column is discovered at runtime and the insert happens only when all
/// required (NOT NULL, no-default) columns can be satisfied; anything else
/// is a diagnostic `Skipped`, never an error and never a partial write.
pub fn reconstruct_codex_sqlite_index(
    codex_home: &Path,
    thread_id: &str,
    forked_from_thread_id: Option<&str>,
    rollout_path: &Path,
    updated_at_epoch: i64,
) -> CodexIndexOutcome {
    let Some(database_path) = newest_codex_state_database(codex_home) else {
        return CodexIndexOutcome::Skipped {
            reason: "no state_<N>.sqlite in the Codex home".to_string(),
        };
    };
    match try_reconstruct_codex_index(
        &database_path,
        thread_id,
        forked_from_thread_id,
        rollout_path,
        updated_at_epoch,
    ) {
        Ok(outcome) => outcome,
        Err(reason) => CodexIndexOutcome::Skipped { reason },
    }
}

fn newest_codex_state_database(codex_home: &Path) -> Option<PathBuf> {
    let mut newest: Option<(u64, PathBuf)> = None;
    for entry in fs::read_dir(codex_home).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(version) = name
            .strip_prefix("state_")
            .and_then(|rest| rest.strip_suffix(".sqlite"))
            .and_then(|digits| digits.parse::<u64>().ok())
        else {
            continue;
        };
        if newest.as_ref().is_none_or(|(best, _)| version > *best) {
            newest = Some((version, entry.path()));
        }
    }
    newest.map(|(_, path)| path)
}

struct SqliteColumn {
    name: String,
    declared_type: String,
    not_null: bool,
    has_default: bool,
    primary_key: bool,
}

fn try_reconstruct_codex_index(
    database_path: &Path,
    thread_id: &str,
    forked_from_thread_id: Option<&str>,
    rollout_path: &Path,
    updated_at_epoch: i64,
) -> Result<CodexIndexOutcome, String> {
    let connection = rusqlite::Connection::open(database_path)
        .map_err(|error| format!("open '{}': {}", database_path.display(), error))?;
    connection
        .busy_timeout(Duration::from_millis(500))
        .map_err(|error| format!("configure state database: {}", error))?;
    let columns = sqlite_table_columns(&connection, "threads")?;
    if columns.is_empty() {
        return Err("state database has no threads table".to_string());
    }

    let mut names = Vec::new();
    let mut values: Vec<rusqlite::types::Value> = Vec::new();
    let mut id_column = None;
    for column in &columns {
        let lower = column.name.to_ascii_lowercase();
        let value: Option<rusqlite::types::Value> = if lower == "id" || lower == "thread_id" {
            id_column = Some(column.name.clone());
            Some(thread_id.to_string().into())
        } else if lower.contains("rollout") || lower.contains("path") {
            Some(rollout_path.to_string_lossy().into_owned().into())
        } else if lower.contains("fork") || lower.contains("parent") {
            forked_from_thread_id.map(|parent| parent.to_string().into())
        } else if lower.contains("created")
            || lower.contains("updated")
            || lower.contains("modified")
            || lower.ends_with("_at")
            || lower.contains("time")
        {
            if column.declared_type.to_ascii_uppercase().contains("INT") {
                Some(updated_at_epoch.into())
            } else {
                Some(
                    DateTime::from_timestamp(updated_at_epoch, 0)
                        .unwrap_or_default()
                        .to_rfc3339()
                        .into(),
                )
            }
        } else if lower.contains("archived") || lower.contains("deleted") {
            column
                .declared_type
                .to_ascii_uppercase()
                .contains("INT")
                .then(|| 0_i64.into())
        } else {
            None
        };
        match value {
            Some(value) => {
                names.push(format!("\"{}\"", column.name.replace('"', "\"\"")));
                values.push(value);
            }
            None => {
                if column.not_null && !column.has_default && !column.primary_key {
                    return Err(format!(
                        "threads column '{}' is required but not understood",
                        column.name
                    ));
                }
            }
        }
    }
    let id_column =
        id_column.ok_or_else(|| "threads table has no recognizable id column".to_string())?;

    let placeholders = (1..=values.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let insert = format!(
        "INSERT OR IGNORE INTO threads ({}) VALUES ({})",
        names.join(", "),
        placeholders
    );
    let changed = connection
        .execute(&insert, rusqlite::params_from_iter(values))
        .map_err(|error| format!("insert thread row: {}", error))?;
    if changed == 0 {
        return Ok(CodexIndexOutcome::AlreadyIndexed);
    }

    // Lineage edges are secondary metadata: attempt them, but never fail the
    // reconstruction over them.
    if let Some(parent) = forked_from_thread_id {
        let _ = insert_codex_spawn_edge(&connection, parent, thread_id);
    }
    let _ = id_column;
    Ok(CodexIndexOutcome::Inserted)
}

fn insert_codex_spawn_edge(
    connection: &rusqlite::Connection,
    parent_thread_id: &str,
    child_thread_id: &str,
) -> Result<(), String> {
    let columns = sqlite_table_columns(connection, "thread_spawn_edges")?;
    let mut parent_column = None;
    let mut child_column = None;
    for column in &columns {
        let lower = column.name.to_ascii_lowercase();
        if lower.contains("parent") || lower.contains("from") {
            parent_column.get_or_insert(column.name.clone());
        } else if lower.contains("child") || lower.contains("to") {
            child_column.get_or_insert(column.name.clone());
        }
    }
    let (Some(parent_column), Some(child_column)) = (parent_column, child_column) else {
        return Err("thread_spawn_edges columns not understood".to_string());
    };
    for column in &columns {
        if column.name != parent_column
            && column.name != child_column
            && column.not_null
            && !column.has_default
            && !column.primary_key
        {
            return Err(format!(
                "thread_spawn_edges column '{}' is required but not understood",
                column.name
            ));
        }
    }
    connection
        .execute(
            &format!(
                "INSERT OR IGNORE INTO thread_spawn_edges (\"{}\", \"{}\") VALUES (?1, ?2)",
                parent_column.replace('"', "\"\""),
                child_column.replace('"', "\"\"")
            ),
            rusqlite::params![parent_thread_id, child_thread_id],
        )
        .map_err(|error| format!("insert spawn edge: {}", error))?;
    Ok(())
}

fn sqlite_table_columns(
    connection: &rusqlite::Connection,
    table: &str,
) -> Result<Vec<SqliteColumn>, String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\"")))
        .map_err(|error| format!("inspect {} schema: {}", table, error))?;
    let rows = statement
        .query_map([], |row| {
            Ok(SqliteColumn {
                name: row.get::<_, String>(1)?,
                declared_type: row.get::<_, String>(2).unwrap_or_default(),
                not_null: row.get::<_, i64>(3)? != 0,
                has_default: row.get::<_, rusqlite::types::Value>(4)?
                    != rusqlite::types::Value::Null,
                primary_key: row.get::<_, i64>(5)? != 0,
            })
        })
        .map_err(|error| format!("read {} schema: {}", table, error))?;
    let mut columns = Vec::new();
    for row in rows {
        columns.push(row.map_err(|error| format!("read {} schema row: {}", table, error))?);
    }
    Ok(columns)
}

fn list_rollout_files(sessions_root: &Path) -> Result<std::collections::BTreeSet<PathBuf>, String> {
    let mut rollouts = std::collections::BTreeSet::new();
    if !sessions_root.exists() {
        return Ok(rollouts);
    }
    for entry in walkdir::WalkDir::new(sessions_root).follow_links(false) {
        let entry = entry.map_err(|error| format!("scan codex sessions: {}", error))?;
        if entry.file_type().is_file()
            && entry.path().extension().is_some_and(|ext| ext == "jsonl")
        {
            rollouts.insert(entry.path().to_path_buf());
        }
    }
    Ok(rollouts)
}

fn find_rollout_by_thread_id(
    sessions_root: &Path,
    thread_id: &str,
) -> Result<Option<PathBuf>, String> {
    for path in list_rollout_files(sessions_root)? {
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains(thread_id))
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn transcript_present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            Ok(metadata.len() > 0)
        }
        Ok(_) => Err(format!(
            "'{}' is not a regular no-follow file",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("inspect '{}': {}", path.display(), error)),
    }
}

fn read_bounded_transcript(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect '{}': {}", path.display(), error))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "'{}' is not a regular no-follow file",
            path.display()
        ));
    }
    if metadata.len() > MAX_TRANSCRIPT_BYTES {
        return Err(format!("'{}' exceeds the transcript limit", path.display()));
    }
    fs::read(path).map_err(|error| format!("read '{}': {}", path.display(), error))
}

fn write_new_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("create '{}': {}", path.display(), error))?;
    file.write_all(bytes)
        .map_err(|error| format!("write '{}': {}", path.display(), error))?;
    file.sync_all()
        .map_err(|error| format!("sync '{}': {}", path.display(), error))
}

fn truncate_for_error(text: &str) -> String {
    const LIMIT: usize = 512;
    let cleaned = text
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(LIMIT)
        .collect::<String>();
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_sync_v3::domain::generated_uuid_v4;
    use std::cell::RefCell;

    struct FakeRunner {
        invocations: RefCell<Vec<ProviderCliInvocation>>,
        behavior: Box<dyn Fn(&ProviderCliInvocation) -> Result<ProviderCliOutput, String>>,
    }

    impl FakeRunner {
        fn new(
            behavior: impl Fn(&ProviderCliInvocation) -> Result<ProviderCliOutput, String> + 'static,
        ) -> Self {
            Self {
                invocations: RefCell::new(Vec::new()),
                behavior: Box::new(behavior),
            }
        }

        fn count(&self) -> usize {
            self.invocations.borrow().len()
        }
    }

    impl ProviderCliRunner for FakeRunner {
        fn run(&self, invocation: &ProviderCliInvocation) -> Result<ProviderCliOutput, String> {
            self.invocations.borrow_mut().push(invocation.clone());
            (self.behavior)(invocation)
        }
    }

    fn ok_output() -> ProviderCliOutput {
        ProviderCliOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn codex_transcript(thread_id: &str, cwd: &str) -> Vec<u8> {
        let mut transcript = format!(
            "{}\n",
            serde_json::json!({
                "timestamp": "2026-07-22T00:00:00Z",
                "type": "session_meta",
                "payload": {"id": thread_id, "cwd": cwd}
            })
        );
        for (index, id) in ["m-a", "m-b"].iter().enumerate() {
            transcript.push_str(&format!(
                "{}\n",
                serde_json::json!({
                    "timestamp": format!("2026-07-22T00:00:0{}Z", index + 1),
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "id": id,
                        "parent_id": if index == 0 { serde_json::Value::Null } else { "m-a".into() },
                        "role": if index == 0 { "user" } else { "assistant" },
                        "content": [{"type": "input_text", "text": "hello"}]
                    }
                })
            ));
        }
        transcript.into_bytes()
    }

    #[test]
    fn generated_uuid_v4_is_provider_native_format() {
        let first = generated_uuid_v4().unwrap();
        let second = generated_uuid_v4().unwrap();
        assert_ne!(first, second);
        assert!(valid_uuid_v4(&first), "{first}");
        assert_eq!(first.as_bytes()[14], b'4', "version nibble");
        assert!(matches!(first.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn claude_project_dir_name_matches_documented_sanitization() {
        assert_eq!(
            claude_project_dir_name(Path::new("/Users/dev/my_app.v2")),
            "-Users-dev-my-app-v2"
        );
    }

    #[test]
    fn claude_fork_invokes_cli_once_and_verifies_the_transcript() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("claude-home");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let project_dir = home
            .join("projects")
            .join(claude_project_dir_name(&project_root));
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("11111111-1111-4111-8111-111111111111.jsonl"), b"src\n")
            .unwrap();
        let new_id = generated_uuid_v4().unwrap();
        let request = ClaudeForkRequest {
            claude_home: home.clone(),
            project_root: project_root.clone(),
            source_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            new_session_id: new_id.clone(),
            binary: PathBuf::from("claude"),
            timeout: Duration::from_secs(5),
        };
        let target = project_dir.join(format!("{new_id}.jsonl"));
        let copy_target = target.clone();
        let copy_source = project_dir.join("11111111-1111-4111-8111-111111111111.jsonl");
        let runner = FakeRunner::new(move |invocation| {
            assert_eq!(
                invocation.args[..5],
                [
                    "--resume".to_string(),
                    "11111111-1111-4111-8111-111111111111".to_string(),
                    "--fork-session".to_string(),
                    "--session-id".to_string(),
                    invocation.args[4].clone(),
                ]
            );
            assert_eq!(invocation.args[5], "-p");
            assert!(invocation
                .envs
                .iter()
                .any(|(key, _)| key == "CLAUDE_CONFIG_DIR"));
            fs::copy(&copy_source, &copy_target).unwrap();
            Ok(ok_output())
        });
        let forked = fork_claude_session(&runner, &request).unwrap();
        assert_eq!(forked.provider_thread_id, new_id);
        assert_eq!(
            forked.forked_from_provider_thread_id,
            "11111111-1111-4111-8111-111111111111"
        );
        assert!(!forked.already_existed);
        assert_eq!(forked.transcript_path, target);
        assert_eq!(runner.count(), 1);

        // Retry after a crash between fork and journaling: the pinned UUID
        // makes the existing transcript proof of completion.
        let retried = fork_claude_session(&runner, &request).unwrap();
        assert!(retried.already_existed);
        assert_eq!(runner.count(), 1, "no second CLI invocation");
    }

    #[test]
    fn claude_fork_rejects_a_silent_cli_that_wrote_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let request = ClaudeForkRequest {
            claude_home: temp.path().join("claude-home"),
            project_root,
            source_session_id: "11111111-1111-4111-8111-111111111111".to_string(),
            new_session_id: generated_uuid_v4().unwrap(),
            binary: PathBuf::from("claude"),
            timeout: Duration::from_secs(5),
        };
        let runner = FakeRunner::new(|_| Ok(ok_output()));
        let error = fork_claude_session(&runner, &request).unwrap_err();
        assert!(error.contains("did not produce"));
    }

    #[test]
    fn claude_fork_requires_a_uuid_session_id() {
        let temp = tempfile::tempdir().unwrap();
        let request = ClaudeForkRequest {
            claude_home: temp.path().join("home"),
            project_root: temp.path().to_path_buf(),
            source_session_id: "src".to_string(),
            new_session_id: "thread-not-a-uuid".to_string(),
            binary: PathBuf::from("claude"),
            timeout: Duration::from_secs(5),
        };
        let runner = FakeRunner::new(|_| Ok(ok_output()));
        assert!(fork_claude_session(&runner, &request)
            .unwrap_err()
            .contains("UUIDv4"));
        assert_eq!(runner.count(), 0);
    }

    fn codex_request(temp: &tempfile::TempDir, allow_cli: bool) -> CodexForkRequest {
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).unwrap();
        let project_root = fs::canonicalize(&project_root).unwrap();
        CodexForkRequest {
            codex_home: temp.path().join("codex-home"),
            project_root: project_root.clone(),
            source_thread_id: "22222222-2222-4222-8222-222222222222".to_string(),
            new_thread_id: generated_uuid_v4().unwrap(),
            source_transcript: codex_transcript(
                "22222222-2222-4222-8222-222222222222",
                project_root.to_str().unwrap(),
            ),
            binary: PathBuf::from("codex"),
            timeout: Duration::from_secs(5),
            allow_cli,
            now: DateTime::parse_from_rfc3339("2026-07-23T10-00-00+00:00".replace('-', ":").as_str())
                .map(|value| value.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        }
    }

    #[test]
    fn codex_cli_fork_is_verified_by_rollout_discovery_and_chain_equality() {
        let temp = tempfile::tempdir().unwrap();
        let mut request = codex_request(&temp, true);
        request.now = Utc::now();
        let sessions = request.codex_home.join("sessions/2026/07/22");
        fs::create_dir_all(&sessions).unwrap();
        let source_rollout = sessions.join(format!(
            "rollout-2026-07-22T00-00-00-{}.jsonl",
            request.source_thread_id
        ));
        fs::write(&source_rollout, &request.source_transcript).unwrap();

        let minted = "33333333-3333-4333-8333-333333333333";
        let forked_bytes = String::from_utf8(request.source_transcript.clone())
            .unwrap()
            .replace(&request.source_thread_id, minted)
            .into_bytes();
        let fork_target = sessions.join(format!("rollout-2026-07-22T01-00-00-{minted}.jsonl"));
        let runner = FakeRunner::new(move |invocation| {
            assert_eq!(invocation.args, vec!["fork", "22222222-2222-4222-8222-222222222222"]);
            assert!(invocation.envs.iter().any(|(key, _)| key == "CODEX_HOME"));
            fs::write(&fork_target, &forked_bytes).unwrap();
            Ok(ok_output())
        });
        let forked = fork_codex_session(&runner, &request).unwrap();
        assert_eq!(forked.provider_thread_id, minted);
        assert_eq!(forked.method, ProviderForkMethod::CodexCli);
        assert_eq!(
            forked.forked_from_provider_thread_id,
            request.source_thread_id
        );
    }

    #[test]
    fn codex_cli_failure_falls_back_to_codec_copy_and_reconstructs_the_index() {
        let temp = tempfile::tempdir().unwrap();
        let mut request = codex_request(&temp, true);
        request.now = Utc::now();
        fs::create_dir_all(&request.codex_home).unwrap();
        install_codex_state_fixture(&request.codex_home);
        let runner = FakeRunner::new(|_| Err("codex binary is not installed".to_string()));

        let forked = fork_codex_session(&runner, &request).unwrap();
        assert_eq!(forked.provider_thread_id, request.new_thread_id);
        assert!(matches!(
            forked.method,
            ProviderForkMethod::CodexCodec {
                index: CodexIndexOutcome::Inserted
            }
        ));
        let bytes = fs::read(&forked.transcript_path).unwrap();
        let analysis = analyze_codex_transcript(&bytes);
        assert_eq!(
            analysis.source_thread_id.as_deref(),
            Some(request.new_thread_id.as_str())
        );
        let source_analysis = analyze_codex_transcript(&request.source_transcript);
        assert_eq!(
            classify_conversation_relation(
                analysis.lineage.chain(),
                source_analysis.lineage.chain()
            ),
            ConversationRelation::Identical,
            "fork copies content exactly",
        );
        let file_name = forked
            .transcript_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(file_name.starts_with("rollout-"));
        assert!(file_name.contains(&request.new_thread_id));

        let connection =
            rusqlite::Connection::open(request.codex_home.join("state_5.sqlite")).unwrap();
        let (rollout, forked_from): (String, String) = connection
            .query_row(
                "SELECT rollout_path, forked_from_id FROM threads WHERE id = ?1",
                [request.new_thread_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(rollout, forked.transcript_path.to_string_lossy());
        assert_eq!(forked_from, request.source_thread_id);
        let edges: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM thread_spawn_edges WHERE parent_thread_id = ?1 AND child_thread_id = ?2",
                [request.source_thread_id.as_str(), request.new_thread_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(edges, 1, "lineage edge recorded");

        // Retry: pinned thread ID resolves to the already-written rollout.
        let retried = fork_codex_session(&runner, &request).unwrap();
        assert!(retried.already_existed);
        assert!(matches!(
            retried.method,
            ProviderForkMethod::CodexCodec {
                index: CodexIndexOutcome::AlreadyIndexed
            }
        ));
    }

    #[test]
    fn sqlite_reconstruction_skips_unknown_schemas_without_writing() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();

        // No database at all: skipped.
        let missing = reconstruct_codex_sqlite_index(
            &home,
            "44444444-4444-4444-8444-444444444444",
            None,
            Path::new("/tmp/rollout.jsonl"),
            1_784_678_410,
        );
        assert!(matches!(missing, CodexIndexOutcome::Skipped { .. }));

        // A NOT NULL column reconstruction cannot satisfy: skipped, no row.
        let connection = rusqlite::Connection::open(home.join("state_9.sqlite")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    mystery_token BLOB NOT NULL
                );",
            )
            .unwrap();
        drop(connection);
        let skipped = reconstruct_codex_sqlite_index(
            &home,
            "44444444-4444-4444-8444-444444444444",
            None,
            Path::new("/tmp/rollout.jsonl"),
            1_784_678_410,
        );
        match skipped {
            CodexIndexOutcome::Skipped { reason } => assert!(reason.contains("mystery_token")),
            other => panic!("expected skip, got {other:?}"),
        }
        let connection = rusqlite::Connection::open(home.join("state_9.sqlite")).unwrap();
        let rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0, "unknown schemas are never written to");
    }

    #[test]
    fn sqlite_reconstruction_targets_the_newest_state_generation() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().to_path_buf();
        for generation in [3_u32, 5] {
            let connection = rusqlite::Connection::open(
                home.join(format!("state_{generation}.sqlite")),
            )
            .unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        rollout_path TEXT,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );",
                )
                .unwrap();
        }
        let outcome = reconstruct_codex_sqlite_index(
            &home,
            "55555555-5555-4555-8555-555555555555",
            None,
            Path::new("/tmp/rollout.jsonl"),
            1_784_678_410,
        );
        assert_eq!(outcome, CodexIndexOutcome::Inserted);
        let newest = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
        let rows: i64 = newest
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        let older = rusqlite::Connection::open(home.join("state_3.sqlite")).unwrap();
        let rows: i64 = older
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0, "only the live generation is reconstructed");
    }

    fn install_codex_state_fixture(codex_home: &Path) {
        let connection =
            rusqlite::Connection::open(codex_home.join("state_5.sqlite")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    forked_from_id TEXT,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    archived INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE thread_spawn_edges (
                    parent_thread_id TEXT NOT NULL,
                    child_thread_id TEXT NOT NULL,
                    PRIMARY KEY (parent_thread_id, child_thread_id)
                );",
            )
            .unwrap();
    }
}
