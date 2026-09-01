//! The typed Claude provider adapter: profile facts, session discovery,
//! bounded history parsing, the versioned lineage codec, materialization
//! target planning, and safe session actions.

pub mod actions;
pub mod codec;
pub mod discovery;
pub mod history;
pub mod materialization;

use super::traits::{
    MaterializationTargetRequest, PlannedProviderTarget, ProviderCliLauncher, ProviderDescriptor,
    SessionActionContext, SessionActionDescriptor, SessionActionOutcome, SessionActionRequest,
    SessionAdapter, SessionAnalysis, SessionDiscovery, SessionDiscoveryContext, SessionEventDetail,
    SessionFileMetadata, SessionFileSummary, SessionTimelineContext, SessionTimelineIndex,
    SessionTimelinePageSlice, SessionTimelineQuery,
};
use crate::app_settings::conversation_file_limit_bytes;
use crate::provider_capture::validated_session_metadata;
use mallard_domain::ProviderKey;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub use codec::{
    analyze_claude_transcript, project_claude_record, CLAUDE_CODEC_VERSION,
    CLAUDE_LINEAGE_CODEC_KEY,
};
pub use materialization::encode_claude_project_path;

pub fn provider_key() -> ProviderKey {
    ProviderKey::parse("claude").expect("static provider key")
}

/// The suffix-window size for the bounded metadata scan, frozen against the
/// 2026-09-01 corpus sweep (docs/DESIGN_CLAUDE_SCAN_TAIL_WINDOW.md): resumed
/// and forked transcripts carry an id transition near the tail and decide
/// within one window; single-id files fall back to the full stream.
const SCAN_TAIL_WINDOW_BYTES: u64 = 256 * 1024;

enum WindowScan {
    Decided(codec::ClaudeSessionIdentityTracker),
    Undecided,
}

fn scan_claude_session_metadata(path: &Path) -> Result<SessionFileMetadata, String> {
    scan_claude_session_metadata_with_window(path, SCAN_TAIL_WINDOW_BYTES)
}

fn scan_claude_session_metadata_with_window(
    path: &Path,
    window_bytes: u64,
) -> Result<SessionFileMetadata, String> {
    crate::sync_metrics::record_file_head_read(path);
    let file = crate::jsonl_tail_io::open_no_follow(path)?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect Claude session '{}': {}", path.display(), error))?;
    if !metadata.is_file() {
        return Err("Claude session transcript is not a regular file".to_string());
    }
    if metadata.len() > conversation_file_limit_bytes() {
        return Err(format!(
            "Claude session exceeds {} bytes",
            conversation_file_limit_bytes()
        ));
    }

    let identity = if metadata.len() > window_bytes {
        drop(file);
        match scan_tail_window(path, window_bytes)? {
            WindowScan::Decided(identity) => identity,
            WindowScan::Undecided => {
                scan_full_stream(crate::jsonl_tail_io::open_no_follow(path)?, path)?
            }
        }
    } else {
        scan_full_stream(file, path)?
    };

    let (session_id, authorized_home) = identity.leaf_identity();
    let session_id = session_id
        .ok_or_else(|| "authoritative Claude leaf session ID was not found".to_string())?;
    let authorized_home = authorized_home
        .ok_or_else(|| "authorized Claude session home was not found".to_string())?;
    let file_session_id = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".jsonl"))
        .ok_or_else(|| "Claude session filename is invalid".to_string())?;
    if file_session_id != session_id {
        return Err("Claude session filename does not match its leaf session ID".to_string());
    }
    validated_session_metadata(session_id, authorized_home, false)
}

/// One bounded read of the transcript tail. The window stands in for the
/// whole file iff every complete in-window chain record carries its own
/// `sessionId` and an id transition is in-window — that places the file's
/// LAST transition, and with it the entire leaf segment, inside the window,
/// so the tracker's leaf identity over the window equals the whole-file
/// result. Anything else is undecidable from the tail (the leaf segment may
/// begin before the window) and falls back to the full stream.
fn scan_tail_window(path: &Path, window_bytes: u64) -> Result<WindowScan, String> {
    let window = crate::jsonl_tail_io::read_suffix_window(path, window_bytes)?;
    let bytes = window.bytes.as_slice();
    // Only complete records participate: drop through the first newline when
    // the window starts mid-file (a boundary-aligned start harmlessly treats
    // one whole record as pre-window), and drop the torn tail — a
    // non-newline-terminated record is not durable yet and cannot change
    // leaf identity.
    let start = if window.window_offset == 0 {
        0
    } else {
        match bytes.iter().position(|byte| *byte == b'\n') {
            Some(position) => position + 1,
            None => return Ok(WindowScan::Undecided),
        }
    };
    let end = bytes[start..]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|position| start + position + 1)
        .unwrap_or(start);
    let mut identity = codec::ClaudeSessionIdentityTracker::default();
    let mut record_count = 0_u64;
    let mut every_chain_record_owns_its_id = true;
    let mut previous_chain_id: Option<String> = None;
    let mut transition_in_window = false;
    for line in bytes[start..end].split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        record_count += 1;
        let Ok(record) = serde_json::from_slice::<codec::ClaudeScanRecord>(line) else {
            continue;
        };
        let Some(parts) = record.into_identity_parts() else {
            continue;
        };
        if parts.is_chain {
            match parts.session_id.as_deref() {
                None => every_chain_record_owns_its_id = false,
                Some(id) => {
                    if previous_chain_id
                        .as_deref()
                        .is_some_and(|previous| previous != id)
                    {
                        transition_in_window = true;
                    }
                    previous_chain_id = Some(id.to_string());
                }
            }
        }
        identity.observe_parts(parts);
    }
    crate::sync_metrics::record_records_parsed(path, record_count);
    // A racing truncation can hand back the whole file (offset zero); that
    // window IS the full read and always decides.
    if window.window_offset == 0 || (every_chain_record_owns_its_id && transition_in_window) {
        Ok(WindowScan::Decided(identity))
    } else {
        Ok(WindowScan::Undecided)
    }
}

/// The unbounded fallback, streaming so parallel refresh workers never hold
/// whole transcripts in memory: undecidable tails — single-id files whose
/// authorized home was established at the head, id-less chain records,
/// chainless files — legitimately need every record.
fn scan_full_stream(
    file: std::fs::File,
    path: &Path,
) -> Result<codec::ClaudeSessionIdentityTracker, String> {
    let mut reader = BufReader::new(file);
    let mut identity = codec::ClaudeSessionIdentityTracker::default();
    let mut bytes_read = 0_u64;
    let mut record_count = 0_usize;
    loop {
        let mut line = Vec::new();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("read Claude session metadata: {}", error))?;
        if read == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(read as u64);
        if bytes_read > conversation_file_limit_bytes() {
            return Err(format!(
                "Claude session exceeds {} bytes",
                conversation_file_limit_bytes()
            ));
        }
        // The provider is allowed to be appending. A non-newline-terminated
        // tail is not a durable record yet and cannot change leaf identity.
        if line.last() != Some(&b'\n') {
            break;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        record_count = record_count.saturating_add(1);
        if record_count > codec::MAX_CLAUDE_RECORDS {
            return Err(format!(
                "Claude session exceeds {} metadata records",
                codec::MAX_CLAUDE_RECORDS
            ));
        }
        if let Ok(record) = serde_json::from_slice::<codec::ClaudeScanRecord>(&line) {
            if let Some(parts) = record.into_identity_parts() {
                identity.observe_parts(parts);
            }
        }
    }
    crate::sync_metrics::record_transcript_bytes_read(path, bytes_read);
    crate::sync_metrics::record_records_parsed(path, record_count as u64);
    Ok(identity)
}

pub struct ClaudeAdapter;

impl ClaudeAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionAdapter for ClaudeAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            key: provider_key(),
            display_name: "Claude".to_string(),
            profile_label: ".claude profile".to_string(),
            default_profile_directory: Some(".claude".to_string()),
            setup_default: true,
            cli_launcher: Some(ProviderCliLauncher {
                program: "claude".to_string(),
                display_name: "Claude Code".to_string(),
            }),
        }
    }

    fn codec_key(&self) -> &str {
        CLAUDE_LINEAGE_CODEC_KEY
    }

    fn codec_version(&self) -> u32 {
        CLAUDE_CODEC_VERSION
    }

    fn timeline_version(&self) -> u32 {
        history::TIMELINE_PARSER_VERSION
    }

    fn supports_tail_cursor(&self) -> bool {
        true
    }

    fn discover(&self, context: &SessionDiscoveryContext<'_>) -> Result<SessionDiscovery, String> {
        discovery::discover(context, |path| self.scan_metadata(path))
    }

    fn scan_metadata(&self, path: &Path) -> Result<SessionFileMetadata, String> {
        scan_claude_session_metadata(path)
    }

    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis {
        codec::analyze_claude_transcript(bytes)
    }

    fn materialization_targets(
        &self,
        request: &MaterializationTargetRequest<'_>,
    ) -> Result<Vec<PlannedProviderTarget>, String> {
        materialization::materialization_targets(request)
    }

    fn state_target(
        &self,
        provider_home: &Path,
        project_root: &str,
        logical_suffix: &str,
    ) -> Result<PathBuf, String> {
        materialization::state_target(provider_home, project_root, logical_suffix)
    }

    fn summarize_file(&self, path: &Path) -> Result<SessionFileSummary, String> {
        history::summarize_file(path)
    }

    fn summarize_bytes(&self, bytes: &[u8]) -> Result<SessionFileSummary, String> {
        history::summarize_bytes(bytes)
    }

    fn build_timeline_index(
        &self,
        path: &Path,
        context: &SessionTimelineContext<'_>,
    ) -> Result<SessionTimelineIndex, String> {
        history::build_timeline_index(path, context)
    }

    fn read_timeline_page(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        query: &SessionTimelineQuery,
    ) -> Result<SessionTimelinePageSlice, String> {
        history::read_timeline_page(path, index, query)
    }

    fn read_event_detail(
        &self,
        path: &Path,
        index: &SessionTimelineIndex,
        event_id: &str,
    ) -> Result<SessionEventDetail, String> {
        history::read_event_detail(path, index, event_id)
    }

    fn available_actions(
        &self,
        context: &SessionActionContext<'_>,
    ) -> Vec<SessionActionDescriptor> {
        actions::available_actions(context)
    }

    fn execute_action(
        &self,
        request: &mut SessionActionRequest<'_>,
    ) -> Result<SessionActionOutcome, String> {
        actions::execute_action(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;

    fn jsonl(records: impl IntoIterator<Item = Value>) -> String {
        records
            .into_iter()
            .map(|record| record.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    #[test]
    fn metadata_scan_rejects_near_match_nested_and_split_fields() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let cwd = temp.path().to_string_lossy().into_owned();
        fs::write(
            &path,
            jsonl([
                serde_json::json!({
                    "type": "system-init",
                    "sessionId": "near-match",
                    "cwd": cwd
                }),
                serde_json::json!({
                    "type": "tool_result",
                    "payload": {"sessionId": "nested-tool-id"}
                }),
                serde_json::json!({"type": "diagnostic", "cwd": cwd}),
                serde_json::json!({
                    "type": "user",
                    "sessionId": "sidechain",
                    "cwd": cwd,
                    "isSidechain": true
                }),
            ]),
        )
        .unwrap();

        let error = scan_claude_session_metadata(&path).unwrap_err();
        assert!(error.contains("authoritative Claude leaf"));
    }

    #[test]
    fn metadata_scan_reads_complete_durable_leaf_record() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("current-session.jsonl");
        let cwd = temp.path().to_string_lossy().into_owned();
        fs::write(
            &path,
            jsonl([serde_json::json!({
                "type": "user",
                "uuid": "user-current",
                "parentUuid": null,
                "sessionId": "current-session",
                "cwd": cwd,
                "message": {
                    "id": "unrelated",
                    "input": {"cwd": "/unrelated"}
                }
            })]),
        )
        .unwrap();

        let metadata = scan_claude_session_metadata(&path).unwrap();
        assert_eq!(metadata.session_id, "current-session");
        assert_eq!(metadata.authorized_home, temp.path());
    }

    /// Corpus gate for docs/DESIGN_CLAUDE_SCAN_TAIL_WINDOW.md. Runs the
    /// shipped scan against an embedded reference implementation of the
    /// pre-cutover `Value` scan — old drift-fallback and last-metadata
    /// semantics included — over the machine's real `~/.claude/projects`,
    /// then sweeps window sizes for the constant freeze. Run with:
    /// `cargo test -p mallard-infrastructure --release \
    ///    claude_scan_corpus_parity -- --ignored --nocapture`
    #[test]
    #[ignore = "reads the machine's real Claude home"]
    fn claude_scan_corpus_parity_and_window_sweep() {
        use std::time::Instant;
        use walkdir::WalkDir;

        #[derive(Default)]
        struct OldTracker {
            last_session_id: Option<String>,
            last_cwd: Option<String>,
            last_metadata_identity: Option<(String, String)>,
            leaf_session_id: Option<String>,
            leaf_cwd: Option<String>,
            leaf_home_session_id: Option<String>,
            leaf_session_home: Option<String>,
            saw_chain_record: bool,
        }

        impl OldTracker {
            fn observe(&mut self, value: &Value) {
                let Some(object) = value.as_object() else {
                    return;
                };
                if codec::is_thinking_only_record(object) {
                    return;
                }
                let session_id = object
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(observed) = &session_id {
                    self.last_session_id = Some(observed.clone());
                }
                let cwd = object
                    .get("cwd")
                    .and_then(Value::as_str)
                    .filter(|cwd| !cwd.is_empty())
                    .map(str::to_string);
                if let Some(observed) = &cwd {
                    self.last_cwd = Some(observed.clone());
                }
                let is_durable_metadata = matches!(
                    object.get("type").and_then(Value::as_str),
                    Some("user") | Some("assistant") | Some("system")
                ) && !object
                    .get("isSidechain")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    && !object
                        .get("isMeta")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                if is_durable_metadata {
                    if let (Some(session_id), Some(cwd)) = (&session_id, &cwd) {
                        self.last_metadata_identity = Some((session_id.clone(), cwd.clone()));
                    }
                }
                if codec::is_chain_record(object) {
                    self.saw_chain_record = true;
                    let effective_session_id =
                        session_id.clone().or_else(|| self.last_session_id.clone());
                    if self.leaf_home_session_id != effective_session_id {
                        self.leaf_home_session_id = effective_session_id;
                        self.leaf_session_home = cwd.clone();
                    } else if self.leaf_session_home.is_none() {
                        self.leaf_session_home = cwd.clone();
                    }
                    self.leaf_session_id = session_id;
                    self.leaf_cwd = cwd;
                }
            }

            fn leaf_identity(&self) -> (Option<String>, Option<String>) {
                if !self.saw_chain_record {
                    return self
                        .last_metadata_identity
                        .clone()
                        .map_or((None, None), |(session_id, cwd)| {
                            (Some(session_id), Some(cwd))
                        });
                }
                let session_id = self
                    .leaf_session_id
                    .clone()
                    .or_else(|| self.last_session_id.clone());
                let cwd = if session_id.is_some()
                    && self.leaf_home_session_id.as_ref() == session_id.as_ref()
                {
                    self.leaf_session_home.clone()
                } else {
                    self.leaf_cwd.clone().or_else(|| self.last_cwd.clone())
                };
                (session_id, cwd)
            }
        }

        fn oracle_scan(path: &Path) -> Result<SessionFileMetadata, String> {
            let file = crate::jsonl_tail_io::open_no_follow(path)?;
            let metadata = file.metadata().map_err(|error| error.to_string())?;
            if !metadata.is_file() || metadata.len() > conversation_file_limit_bytes() {
                return Err("bounds".to_string());
            }
            let mut reader = BufReader::new(file);
            let mut identity = OldTracker::default();
            let mut record_count = 0_usize;
            loop {
                let mut line = Vec::new();
                let read = reader
                    .read_until(b'\n', &mut line)
                    .map_err(|error| error.to_string())?;
                if read == 0 {
                    break;
                }
                if line.last() != Some(&b'\n') {
                    break;
                }
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                record_count += 1;
                if record_count > codec::MAX_CLAUDE_RECORDS {
                    return Err("too many records".to_string());
                }
                if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                    identity.observe(&value);
                }
            }
            let (session_id, cwd) = identity.leaf_identity();
            let session_id = session_id.ok_or("no leaf session id")?;
            let cwd = cwd.ok_or("no leaf cwd")?;
            let file_session_id = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".jsonl"))
                .ok_or("bad filename")?;
            if file_session_id != session_id {
                return Err("filename mismatch".to_string());
            }
            validated_session_metadata(session_id, cwd, false)
        }

        let Some(home) = std::env::var_os("HOME") else {
            println!("no HOME; skipping");
            return;
        };
        let projects = PathBuf::from(home).join(".claude").join("projects");
        if !projects.is_dir() {
            println!("no Claude corpus at {}; skipping", projects.display());
            return;
        }
        let files: Vec<PathBuf> = WalkDir::new(&projects)
            .follow_links(false)
            .min_depth(2)
            .max_depth(3)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_type().is_file()
                    && entry.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl")
            })
            .map(|entry| entry.into_path())
            .collect();
        let total_bytes: u64 = files
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum();
        println!(
            "corpus: {} files, {:.2} GB",
            files.len(),
            total_bytes as f64 / 1e9
        );

        let oracle_start = Instant::now();
        let oracle_results: Vec<_> = files.iter().map(|path| oracle_scan(path)).collect();
        let oracle_wall = oracle_start.elapsed();

        let mut both_ok_mismatch = 0_usize;
        let mut old_ok_new_err = 0_usize;
        let mut old_err_new_ok = 0_usize;
        let mut both_err = 0_usize;
        let mut both_ok = 0_usize;
        for (path, oracle) in files.iter().zip(&oracle_results) {
            let new = scan_claude_session_metadata(path);
            match (oracle, &new) {
                (Ok(old), Ok(new)) => {
                    if old.session_id == new.session_id
                        && old.authorized_home == new.authorized_home
                    {
                        both_ok += 1;
                    } else {
                        both_ok_mismatch += 1;
                        println!(
                            "MISMATCH {}: old=({}, {}) new=({}, {})",
                            path.display(),
                            old.session_id,
                            old.authorized_home.display(),
                            new.session_id,
                            new.authorized_home.display()
                        );
                    }
                }
                (Ok(old), Err(error)) => {
                    old_ok_new_err += 1;
                    println!(
                        "DOCTRINE DIVERGENCE {}: old=({}, {}) new error: {}",
                        path.display(),
                        old.session_id,
                        old.authorized_home.display(),
                        error
                    );
                }
                (Err(error), Ok(new)) => {
                    old_err_new_ok += 1;
                    println!(
                        "ACCEPTANCE DIVERGENCE {}: old error '{}' new=({}, {})",
                        path.display(),
                        error,
                        new.session_id,
                        new.authorized_home.display()
                    );
                }
                (Err(_), Err(_)) => both_err += 1,
            }
        }
        println!(
            "parity: {both_ok} identical, {both_err} both-rejected, \
             {old_ok_new_err} doctrine divergences, {old_err_new_ok} acceptance divergences"
        );
        println!("oracle wall: {:.3} s", oracle_wall.as_secs_f64());

        for window in [64_u64 * 1024, 256 * 1024, 1024 * 1024, 4 * 1024 * 1024] {
            let mut decided = 0_usize;
            let mut bytes_read = 0_u64;
            for path in &files {
                let file_len = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
                if file_len <= window {
                    bytes_read += file_len;
                    continue;
                }
                bytes_read += window;
                match scan_tail_window(path, window) {
                    Ok(WindowScan::Decided(_)) => decided += 1,
                    _ => bytes_read += file_len,
                }
            }
            let timed = Instant::now();
            for path in &files {
                let _ = scan_claude_session_metadata_with_window(path, window);
            }
            let wall = timed.elapsed();
            println!(
                "window {:>4} KiB: {decided}/{} large files decided, \
                 {:.2} MB read, wall {:.3} s",
                window / 1024,
                files
                    .iter()
                    .filter(|path| {
                        fs::metadata(path).map(|meta| meta.len()).unwrap_or(0) > window
                    })
                    .count(),
                bytes_read as f64 / 1e6,
                wall.as_secs_f64()
            );
        }

        assert_eq!(both_ok_mismatch, 0, "both-Ok results must be identical");
    }

    fn chain_line(uuid: &str, session: &str, cwd: &std::path::Path, text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "uuid": uuid,
            "sessionId": session,
            "cwd": cwd,
            "message": {"role": "user", "content": text}
        })
        .to_string()
    }

    #[test]
    fn window_scan_decides_resumed_files_without_reading_the_head() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let path = temp.path().join("leaf-session.jsonl");
        // A head past MAX_CLAUDE_RECORDS: the full stream refuses such a
        // file, so an Ok result proves the tail window decided on its own.
        let mut lines = Vec::new();
        for index in 0..=codec::MAX_CLAUDE_RECORDS {
            lines.push(chain_line(
                &format!("u-{index}"),
                "original-session",
                &home,
                "filler",
            ));
        }
        lines.push(chain_line("a-leaf", "leaf-session", &home, "resumed"));
        lines.push(chain_line(
            "u-drift",
            "leaf-session",
            &home.join("src-tauri"),
            "after cd",
        ));
        fs::write(&path, lines.join("\n") + "\n").unwrap();

        let metadata = scan_claude_session_metadata_with_window(&path, 64 * 1024).unwrap();
        assert_eq!(metadata.session_id, "leaf-session");
        assert_eq!(metadata.authorized_home, home);

        // The same bytes through the stream path still enforce the record
        // cap: the window acceptance is deliberate, not a lost bound.
        let error = scan_claude_session_metadata_with_window(&path, u64::MAX).unwrap_err();
        assert!(error.contains("metadata records"), "{error}");
    }

    #[test]
    fn window_scan_falls_back_for_single_id_files() {
        // One session id throughout: the tail cannot prove where the leaf
        // segment began, so the scan must fall back to the full stream and
        // return the head-established authorized home — never the drift cwd
        // that fills the tail.
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let path = temp.path().join("single-session.jsonl");
        let mut lines = vec![chain_line("u-0", "single-session", &home, "launch")];
        for index in 1..1500 {
            lines.push(chain_line(
                &format!("u-{index}"),
                "single-session",
                &home.join("deep").join("drift"),
                "padding padding padding padding padding",
            ));
        }
        fs::write(&path, lines.join("\n") + "\n").unwrap();

        let metadata = scan_claude_session_metadata_with_window(&path, 8 * 1024).unwrap();
        assert_eq!(metadata.session_id, "single-session");
        assert_eq!(metadata.authorized_home, home);
    }

    #[test]
    fn window_scan_ignores_torn_tails_and_thinking_only_rows() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let path = temp.path().join("leaf-session.jsonl");
        let mut lines = Vec::new();
        for index in 0..400 {
            lines.push(chain_line(
                &format!("u-{index}"),
                "original-session",
                &home,
                "filler filler filler filler",
            ));
        }
        lines.push(chain_line("a-leaf", "leaf-session", &home, "resumed"));
        // A thinking-only assistant row is invisible to identity even when
        // it carries a foreign session id and a drift cwd.
        lines.push(
            serde_json::json!({
                "type": "assistant",
                "uuid": "a-thinking",
                "sessionId": "foreign-session",
                "cwd": home.join("elsewhere"),
                "message": {"content": [{"type": "thinking", "thinking": "x"}]}
            })
            .to_string(),
        );
        let mut bytes = (lines.join("\n") + "\n").into_bytes();
        bytes.extend_from_slice(b"{\"type\":\"user\",\"uuid\":\"torn");
        fs::write(&path, bytes).unwrap();

        let metadata = scan_claude_session_metadata_with_window(&path, 8 * 1024).unwrap();
        assert_eq!(metadata.session_id, "leaf-session");
        assert_eq!(metadata.authorized_home, home);
    }

    #[test]
    fn window_scan_falls_back_when_the_window_holds_no_complete_record() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let path = temp.path().join("big-record.jsonl");
        fs::write(
            &path,
            chain_line("u-big", "big-record", &home, &"x".repeat(64 * 1024)) + "\n",
        )
        .unwrap();

        let metadata = scan_claude_session_metadata_with_window(&path, 4 * 1024).unwrap();
        assert_eq!(metadata.session_id, "big-record");
        assert_eq!(metadata.authorized_home, home);
    }

    #[test]
    fn window_scan_falls_back_when_a_chain_record_lacks_its_own_id() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let path = temp.path().join("leaf-session.jsonl");
        let mut lines = Vec::new();
        for index in 0..400 {
            lines.push(chain_line(
                &format!("u-{index}"),
                "original-session",
                &home,
                "filler filler filler filler",
            ));
        }
        lines.push(
            serde_json::json!({
                "type": "user",
                "uuid": "u-anonymous",
                "cwd": home.join("elsewhere"),
                "message": {"role": "user", "content": "no session id"}
            })
            .to_string(),
        );
        lines.push(chain_line("a-leaf", "leaf-session", &home, "resumed"));
        fs::write(&path, lines.join("\n") + "\n").unwrap();

        let metadata = scan_claude_session_metadata_with_window(&path, 8 * 1024).unwrap();
        assert_eq!(metadata.session_id, "leaf-session");
        assert_eq!(metadata.authorized_home, home);
    }

    #[test]
    fn metadata_scan_uses_resumed_leaf_identity_and_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let original_cwd = temp.path().join("original");
        let resumed_cwd = temp.path().join("resumed");
        let resumed_turn_cwd = resumed_cwd.join("src-tauri");
        fs::create_dir_all(&original_cwd).unwrap();
        fs::create_dir_all(&resumed_turn_cwd).unwrap();
        let path = temp.path().join("resumed-session.jsonl");
        fs::write(
            &path,
            jsonl([
                serde_json::json!({
                    "type": "user",
                    "uuid": "user-original",
                    "parentUuid": null,
                    "sessionId": "original-session",
                    "cwd": original_cwd,
                    "message": {"role": "user", "content": "original"}
                }),
                serde_json::json!({
                    "type": "assistant",
                    "uuid": "assistant-resumed",
                    "parentUuid": "user-original",
                    "sessionId": "resumed-session",
                    "cwd": resumed_cwd,
                    "message": {"role": "assistant", "content": [{
                        "type": "text", "text": "resumed"
                    }]}
                }),
                serde_json::json!({
                    "type": "user",
                    "uuid": "user-after-cd",
                    "parentUuid": "assistant-resumed",
                    "sessionId": "resumed-session",
                    "cwd": resumed_turn_cwd,
                    "message": {"role": "user", "content": "continue"}
                }),
            ]),
        )
        .unwrap();

        let metadata = scan_claude_session_metadata(&path).unwrap();
        assert_eq!(metadata.session_id, "resumed-session");
        assert_eq!(metadata.authorized_home, resumed_cwd);

        let mismatched = temp.path().join("wrong-session.jsonl");
        fs::copy(&path, &mismatched).unwrap();
        assert!(scan_claude_session_metadata(&mismatched)
            .unwrap_err()
            .contains("filename does not match"));
    }
}
