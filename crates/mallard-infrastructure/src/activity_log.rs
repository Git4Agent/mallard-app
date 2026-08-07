//! Structured, local-only Mallard activity logging: the JSONL store.
//!
//! Activity entries are written as bounded JSON Lines segments below
//! `~/.mallard/logs`. The desktop crate emits the same entry to the frontend,
//! so the live drawer and the retained history always share one schema.

use crate::persistence::{write_json_atomic, LocalStateRepository};
use chrono::{DateTime, Days, NaiveDate, Utc};
use mallard_domain::generated_named_id;
use serde::{Deserialize, Serialize};
#[cfg(not(test))]
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const ACTIVITY_LOG_SCHEMA: u32 = 1;
const ACTIVITY_LOG_POLICY_SCHEMA: u32 = 1;
const DEFAULT_RETENTION_DAYS: u32 = 30;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024;
const MAX_SEGMENT_BYTES: u64 = 10 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 16 * 1024;
const MAX_QUERY_LIMIT: usize = 2_000;
const POLICY_FILE: &str = "_policy.json";
const LOCK_FILE: &str = "_activity.lock";
const LOG_PREFIX: &str = "activity-";
const LOG_SUFFIX: &str = ".jsonl";

static LOG_IO_LOCK: Mutex<()> = Mutex::new(());
#[cfg(not(test))]
static CLEANUP_STATE: Mutex<Option<BTreeMap<PathBuf, (String, u32)>>> = Mutex::new(None);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ActivityLogLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl ActivityLogLevel {
    /// Lenient parser for the internal string-level API ("ok"/"warn" are the
    /// short forms call sites use today; anything unknown reads as info).
    fn parse_lenient(value: &str) -> Self {
        match value {
            "ok" | "success" => Self::Success,
            "warn" | "warning" => Self::Warning,
            "error" => Self::Error,
            _ => Self::Info,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ActivityLogType {
    Push,
    Pull,
    Repair,
    Storage,
    Configuration,
    History,
    System,
}

impl ActivityLogType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Pull => "pull",
            Self::Repair => "repair",
            Self::Storage => "storage",
            Self::Configuration => "configuration",
            Self::History => "history",
            Self::System => "system",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivityLogContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

impl ActivityLogContext {
    fn sanitized(mut self) -> Self {
        for value in [
            &mut self.project_name,
            &mut self.storage_name,
            &mut self.resource_id,
        ] {
            if let Some(current) = value.take() {
                *value = Some(sanitize_message(&current, 512));
            }
        }
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ActivityLogEntry {
    pub schema: u32,
    pub id: String,
    pub ts: u64,
    pub level: ActivityLogLevel,
    #[serde(rename = "type")]
    pub log_type: ActivityLogType,
    pub event: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "ActivityLogContext::is_empty")]
    pub context: ActivityLogContext,
}

impl ActivityLogContext {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

impl ActivityLogEntry {
    pub fn new(
        log_type: ActivityLogType,
        level: &str,
        event: &str,
        message: &str,
        run_id: Option<String>,
        context: ActivityLogContext,
    ) -> Self {
        let ts = now_millis();
        let id = generated_named_id("event")
            .unwrap_or_else(|_| format!("event-{ts:016x}-{:08x}", std::process::id()));
        Self {
            schema: ACTIVITY_LOG_SCHEMA,
            id,
            ts,
            level: ActivityLogLevel::parse_lenient(level),
            log_type,
            event: normalize_event_name(event, log_type, level),
            message: sanitize_message(message, 8 * 1024),
            run_id,
            context: context.sanitized(),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != ACTIVITY_LOG_SCHEMA {
            return Err(format!("unsupported activity log schema {}", self.schema));
        }
        if self.id.is_empty() || self.id.len() > 128 {
            return Err("activity log event ID is invalid".to_string());
        }
        if self.event.is_empty()
            || self.event.len() > 128
            || !self.event.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
        {
            return Err(format!("activity event name is invalid: '{}'", self.event));
        }
        if self.message.len() > 8 * 1024 {
            return Err("activity log message is too large".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ActivityLogPolicy {
    pub schema: u32,
    pub retention_days: u32,
    pub max_total_bytes: u64,
}

impl Default for ActivityLogPolicy {
    fn default() -> Self {
        Self {
            schema: ACTIVITY_LOG_POLICY_SCHEMA,
            retention_days: DEFAULT_RETENTION_DAYS,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
        }
    }
}

impl ActivityLogPolicy {
    fn validate(&self) -> Result<(), String> {
        if self.schema != ACTIVITY_LOG_POLICY_SCHEMA {
            return Err(format!(
                "unsupported activity log policy schema {}",
                self.schema
            ));
        }
        if !(1..=3_650).contains(&self.retention_days) {
            return Err("log retention must be between 1 and 3650 days".to_string());
        }
        if !(1024 * 1024..=10 * 1024 * 1024 * 1024).contains(&self.max_total_bytes) {
            return Err("maximum log storage must be between 1 MB and 10 GB".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ActivityLogStats {
    pub total_bytes: u64,
    pub file_count: usize,
    pub oldest_ts: Option<u64>,
    pub newest_ts: Option<u64>,
    pub policy: ActivityLogPolicy,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ActivityLogQuery {
    #[serde(default)]
    pub types: Vec<ActivityLogType>,
    #[serde(default)]
    pub levels: Vec<ActivityLogLevel>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ActivityLogPage {
    pub entries: Vec<ActivityLogEntry>,
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ActivityLogCleanupRequest {
    #[serde(default)]
    pub delete_all: bool,
    #[serde(default)]
    pub older_than_days: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ActivityLogCleanupResult {
    pub removed_files: usize,
    pub reclaimed_bytes: u64,
    pub stats: ActivityLogStats,
}

#[derive(Clone, Debug)]
pub struct ActivityLogStore {
    root: PathBuf,
}

impl ActivityLogStore {
    #[cfg(test)]
    pub fn from_home_dir(home: impl Into<PathBuf>) -> Result<Self, String> {
        let repository = LocalStateRepository::from_home_dir(home)?;
        Self::from_repository(&repository)
    }

    pub fn from_repository(repository: &LocalStateRepository) -> Result<Self, String> {
        let store = Self {
            root: repository.root().join("logs"),
        };
        store.ensure_root()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn append(&self, entry: &ActivityLogEntry) -> Result<(), String> {
        entry.validate()?;
        let mut bytes = serde_json::to_vec(entry)
            .map_err(|error| format!("serialize activity log entry: {error}"))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_ENTRY_BYTES {
            return Err("activity log entry exceeds the 16 KB limit".to_string());
        }
        let _guard = log_io_guard()?;
        self.ensure_root()?;
        let _process_guard = self.lock_file()?;
        let day = day_for_timestamp(entry.ts)?;
        let path = self.append_path(&day, bytes.len() as u64)?;
        reject_symlink(&path)?;
        let existed = path.exists();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| format!("open activity log '{}': {error}", path.display()))?;
        if !existed {
            secure_file(&path)?;
        }
        file.write_all(&bytes)
            .and_then(|_| file.flush())
            .map_err(|error| format!("append activity log '{}': {error}", path.display()))
    }

    pub fn query(&self, query: &ActivityLogQuery) -> Result<ActivityLogPage, String> {
        let _guard = log_io_guard()?;
        self.ensure_root()?;
        let _process_guard = self.lock_file()?;
        let limit = query.limit.unwrap_or(500).clamp(1, MAX_QUERY_LIMIT);
        let cursor = query.cursor.as_deref().map(parse_cursor).transpose()?;
        let search = query
            .search
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        let mut entries = Vec::new();
        for record in self.log_files()? {
            let file = File::open(&record.path).map_err(|error| {
                format!("open activity log '{}': {error}", record.path.display())
            })?;
            for line in BufReader::new(file).lines() {
                let Ok(line) = line else { continue };
                if line.len() > MAX_ENTRY_BYTES {
                    continue;
                }
                let Ok(entry) = serde_json::from_str::<ActivityLogEntry>(&line) else {
                    continue;
                };
                if entry.validate().is_err()
                    || (!query.types.is_empty() && !query.types.contains(&entry.log_type))
                    || (!query.levels.is_empty() && !query.levels.contains(&entry.level))
                    || search.as_ref().is_some_and(|needle| {
                        !entry.message.to_lowercase().contains(needle)
                            && !entry.event.to_lowercase().contains(needle)
                    })
                    || cursor
                        .as_ref()
                        .is_some_and(|cursor| !entry_is_before(&entry, cursor))
                {
                    continue;
                }
                entries.push(entry);
            }
        }
        entries.sort_by(|left, right| right.ts.cmp(&left.ts).then_with(|| right.id.cmp(&left.id)));
        let has_more = entries.len() > limit;
        entries.truncate(limit);
        let next_cursor = has_more
            .then(|| entries.last().map(format_cursor))
            .flatten();
        entries.reverse();
        Ok(ActivityLogPage {
            entries,
            next_cursor,
        })
    }

    #[cfg(test)]
    pub fn load_policy(&self) -> Result<ActivityLogPolicy, String> {
        let _guard = log_io_guard()?;
        self.ensure_root()?;
        let _process_guard = self.lock_file()?;
        self.load_policy_unlocked()
    }

    pub fn save_policy(&self, policy: &ActivityLogPolicy) -> Result<(), String> {
        policy.validate()?;
        let _guard = log_io_guard()?;
        self.ensure_root()?;
        let _process_guard = self.lock_file()?;
        self.save_policy_unlocked(policy)
    }

    pub fn stats(&self) -> Result<ActivityLogStats, String> {
        let _guard = log_io_guard()?;
        self.ensure_root()?;
        let _process_guard = self.lock_file()?;
        self.stats_unlocked()
    }

    pub fn cleanup(
        &self,
        request: &ActivityLogCleanupRequest,
    ) -> Result<ActivityLogCleanupResult, String> {
        self.cleanup_at(request, now_millis())
    }

    fn cleanup_at(
        &self,
        request: &ActivityLogCleanupRequest,
        now: u64,
    ) -> Result<ActivityLogCleanupResult, String> {
        let _guard = log_io_guard()?;
        self.ensure_root()?;
        let _process_guard = self.lock_file()?;
        let policy = self.load_policy_unlocked()?;
        let mut files = self.log_files()?;
        files.sort_by(|left, right| left.name.cmp(&right.name));
        let current_day = day_for_timestamp(now)?;
        let active_name = files
            .iter()
            .rev()
            .find(|record| record.day.format("%Y-%m-%d").to_string() == current_day)
            .map(|record| record.name.clone());
        let mut removed_files = 0_usize;
        let mut reclaimed_bytes = 0_u64;

        if request.delete_all {
            for record in files {
                remove_log_file(&record.path)?;
                removed_files += 1;
                reclaimed_bytes = reclaimed_bytes.saturating_add(record.size);
            }
        } else {
            let retention_days = request.older_than_days.unwrap_or(policy.retention_days);
            if !(1..=3_650).contains(&retention_days) {
                return Err("cleanup age must be between 1 and 3650 days".to_string());
            }
            let cutoff = now.saturating_sub(retention_days as u64 * 24 * 60 * 60 * 1_000);
            let mut kept = Vec::new();
            for record in files {
                let expires = record.day_end_ms()? <= cutoff;
                if expires && active_name.as_deref() != Some(record.name.as_str()) {
                    remove_log_file(&record.path)?;
                    removed_files += 1;
                    reclaimed_bytes = reclaimed_bytes.saturating_add(record.size);
                } else {
                    kept.push(record);
                }
            }
            let mut total = kept.iter().map(|record| record.size).sum::<u64>();
            for record in kept {
                if total <= policy.max_total_bytes {
                    break;
                }
                if active_name.as_deref() == Some(record.name.as_str()) {
                    continue;
                }
                remove_log_file(&record.path)?;
                total = total.saturating_sub(record.size);
                removed_files += 1;
                reclaimed_bytes = reclaimed_bytes.saturating_add(record.size);
            }
        }

        Ok(ActivityLogCleanupResult {
            removed_files,
            reclaimed_bytes,
            stats: self.stats_unlocked()?,
        })
    }

    #[cfg(not(test))]
    pub fn maybe_cleanup(&self, day: &str) {
        let should_clean = CLEANUP_STATE
            .lock()
            .ok()
            .map(|mut state| {
                let roots = state.get_or_insert_with(BTreeMap::new);
                let current = roots
                    .entry(self.root.clone())
                    .or_insert_with(|| (String::new(), 100));
                let due = current.0 != day || current.1 >= 100;
                if due {
                    *current = (day.to_string(), 0);
                } else {
                    current.1 = current.1.saturating_add(1);
                }
                due
            })
            .unwrap_or(false);
        if should_clean {
            let _ = self.cleanup(&ActivityLogCleanupRequest::default());
        }
    }

    fn append_path(&self, day: &str, incoming_bytes: u64) -> Result<PathBuf, String> {
        let mut segment = 0_u32;
        let mut existing: Option<(u32, PathBuf, u64)> = None;
        for record in self.log_files()? {
            if record.day.format("%Y-%m-%d").to_string() != day {
                continue;
            }
            if existing
                .as_ref()
                .is_none_or(|(current, _, _)| record.segment > *current)
            {
                existing = Some((record.segment, record.path, record.size));
            }
        }
        if let Some((current, path, size)) = existing {
            if size.saturating_add(incoming_bytes) <= MAX_SEGMENT_BYTES {
                return Ok(path);
            }
            segment = current.saturating_add(1);
        }
        Ok(self.root.join(log_file_name(day, segment)))
    }

    fn log_files(&self) -> Result<Vec<LogFileRecord>, String> {
        let mut files = Vec::new();
        let entries = fs::read_dir(&self.root)
            .map_err(|error| format!("list activity logs '{}': {error}", self.root.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("list activity log entry: {error}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some((day, segment)) = parse_log_file_name(&name) else {
                continue;
            };
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("inspect activity log '{}': {error}", path.display()))?;
            if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                return Err(format!(
                    "activity log '{}' is not a real file",
                    path.display()
                ));
            }
            files.push(LogFileRecord {
                path,
                name,
                day,
                segment,
                size: metadata.len(),
            });
        }
        files.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(files)
    }

    fn stats_unlocked(&self) -> Result<ActivityLogStats, String> {
        let files = self.log_files()?;
        let mut oldest_ts = None;
        let mut newest_ts = None;
        for record in &files {
            let file = File::open(&record.path).map_err(|error| {
                format!("open activity log '{}': {error}", record.path.display())
            })?;
            for line in BufReader::new(file).lines() {
                let Ok(line) = line else { continue };
                let Ok(entry) = serde_json::from_str::<ActivityLogEntry>(&line) else {
                    continue;
                };
                oldest_ts = Some(oldest_ts.map_or(entry.ts, |current: u64| current.min(entry.ts)));
                newest_ts = Some(newest_ts.map_or(entry.ts, |current: u64| current.max(entry.ts)));
            }
        }
        Ok(ActivityLogStats {
            total_bytes: files.iter().map(|record| record.size).sum(),
            file_count: files.len(),
            oldest_ts,
            newest_ts,
            policy: self.load_policy_unlocked()?,
        })
    }

    fn policy_path(&self) -> PathBuf {
        self.root.join(POLICY_FILE)
    }

    fn lock_file(&self) -> Result<File, String> {
        let path = self.root.join(LOCK_FILE);
        reject_symlink(&path)?;
        let existed = path.exists();
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|error| format!("open activity log lock '{}': {error}", path.display()))?;
        if !existed {
            secure_file(&path)?;
        }
        file.lock().map_err(|error| {
            format!("lock activity log store '{}': {error}", self.root.display())
        })?;
        Ok(file)
    }

    fn load_policy_unlocked(&self) -> Result<ActivityLogPolicy, String> {
        let path = self.policy_path();
        reject_symlink(&path)?;
        let policy = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("parse log policy '{}': {error}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ActivityLogPolicy::default()
            }
            Err(error) => return Err(format!("read log policy '{}': {error}", path.display())),
        };
        policy.validate()?;
        Ok(policy)
    }

    fn save_policy_unlocked(&self, policy: &ActivityLogPolicy) -> Result<(), String> {
        let path = self.policy_path();
        reject_symlink(&path)?;
        write_json_atomic(&self.root, &path, policy, 4 * 1024)
    }

    fn ensure_root(&self) -> Result<(), String> {
        let parent = self
            .root
            .parent()
            .ok_or_else(|| format!("activity log root '{}' has no parent", self.root.display()))?;
        ensure_real_directory(parent)?;
        match fs::symlink_metadata(&self.root) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => {
                return Err(format!(
                    "activity log directory '{}' is not a real directory",
                    self.root.display()
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&self.root) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        ensure_real_directory(&self.root)?;
                    }
                    Err(error) => {
                        return Err(format!(
                            "create activity log directory '{}': {error}",
                            self.root.display()
                        ));
                    }
                }
            }
            Err(error) => {
                return Err(format!(
                    "inspect activity log directory '{}': {error}",
                    self.root.display()
                ))
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    format!(
                        "secure activity log directory '{}': {error}",
                        self.root.display()
                    )
                },
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct LogFileRecord {
    path: PathBuf,
    name: String,
    day: NaiveDate,
    segment: u32,
    size: u64,
}

impl LogFileRecord {
    fn day_end_ms(&self) -> Result<u64, String> {
        let next = self
            .day
            .checked_add_days(Days::new(1))
            .ok_or_else(|| "activity log date overflow".to_string())?;
        Ok(next
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "activity log date is invalid".to_string())?
            .and_utc()
            .timestamp_millis()
            .max(0) as u64)
    }
}

fn log_io_guard() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    LOG_IO_LOCK
        .lock()
        .map_err(|_| "activity log lock is poisoned".to_string())
}

pub fn day_for_timestamp(timestamp_ms: u64) -> Result<String, String> {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms as i64)
        .map(|value| value.format("%Y-%m-%d").to_string())
        .ok_or_else(|| "activity log timestamp is outside the supported range".to_string())
}

fn log_file_name(day: &str, segment: u32) -> String {
    format!("{LOG_PREFIX}{day}-{segment:03}{LOG_SUFFIX}")
}

fn parse_log_file_name(name: &str) -> Option<(NaiveDate, u32)> {
    let body = name.strip_prefix(LOG_PREFIX)?.strip_suffix(LOG_SUFFIX)?;
    let (day, segment) = body.rsplit_once('-')?;
    if segment.len() < 3 || !segment.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((
        NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?,
        segment.parse().ok()?,
    ))
}

fn format_cursor(entry: &ActivityLogEntry) -> String {
    format!("{}:{}", entry.ts, entry.id)
}

fn parse_cursor(value: &str) -> Result<(u64, String), String> {
    let (timestamp, id) = value
        .split_once(':')
        .ok_or_else(|| "activity log cursor is invalid".to_string())?;
    let timestamp = timestamp
        .parse::<u64>()
        .map_err(|_| "activity log cursor timestamp is invalid".to_string())?;
    if id.is_empty() || id.len() > 128 {
        return Err("activity log cursor event ID is invalid".to_string());
    }
    Ok((timestamp, id.to_string()))
}

fn entry_is_before(entry: &ActivityLogEntry, cursor: &(u64, String)) -> bool {
    entry.ts < cursor.0 || (entry.ts == cursor.0 && entry.id.as_str() < cursor.1.as_str())
}

pub fn normalized_level_name(level: &str) -> &'static str {
    match ActivityLogLevel::parse_lenient(level) {
        ActivityLogLevel::Info => "info",
        ActivityLogLevel::Success => "success",
        ActivityLogLevel::Warning => "warning",
        ActivityLogLevel::Error => "error",
    }
}

fn normalize_event_name(event: &str, log_type: ActivityLogType, level: &str) -> String {
    let event = event.trim().to_ascii_lowercase();
    if !event.is_empty()
        && event.len() <= 128
        && event.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        event
    } else {
        format!("{}.{}", log_type.as_str(), normalized_level_name(level))
    }
}

fn sanitize_message(message: &str, max_bytes: usize) -> String {
    let mut output = message.replace('\0', "�");
    for marker in [
        "authorization:",
        "bearer ",
        "access_key_id=",
        "access_key_id:",
        "aws_access_key_id=",
        "aws_secret_access_key=",
        "secret_access_key=",
        "secret_access_key:",
        "password=",
        "password:",
        "token=",
        "token:",
        "x-amz-signature=",
        "x-amz-credential=",
        "x-amz-security-token=",
    ] {
        redact_after_marker(&mut output, marker);
    }
    truncate_utf8(&mut output, max_bytes);
    output
}

fn redact_after_marker(value: &mut String, marker: &str) {
    let mut offset = 0;
    loop {
        let lower = value.to_ascii_lowercase();
        let Some(relative_position) = lower.get(offset..).and_then(|tail| tail.find(marker)) else {
            break;
        };
        let position = offset + relative_position;
        let mut start = position + marker.len();
        while start < value.len() && value.as_bytes()[start].is_ascii_whitespace() {
            start += 1;
        }
        if lower
            .get(start..)
            .is_some_and(|tail| tail.starts_with("bearer "))
        {
            start += "bearer ".len();
        }
        while start < value.len() && matches!(value.as_bytes()[start], b'\'' | b'\"') {
            start += 1;
        }
        if value
            .get(start..)
            .is_some_and(|tail| tail.starts_with("[redacted]"))
        {
            offset = start + "[redacted]".len();
            continue;
        }
        let mut end = start;
        while end < value.len()
            && !matches!(
                value.as_bytes()[end],
                b' ' | b'\t' | b'\r' | b'\n' | b'&' | b',' | b'\'' | b'\"'
            )
        {
            end += 1;
        }
        if end == start {
            break;
        }
        value.replace_range(start..end, "[redacted]");
        offset = start + "[redacted]".len();
    }
}

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let suffix = "… [truncated]";
    let mut end = max_bytes.saturating_sub(suffix.len());
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value.truncate(end);
    value.push_str(suffix);
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "activity log path traverses symlink '{}'",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "inspect activity log path '{}': {error}",
            path.display()
        )),
    }
}

fn ensure_real_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect directory '{}': {error}", path.display()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("'{}' is not a real directory", path.display()));
    }
    Ok(())
}

fn secure_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("secure activity log '{}': {error}", path.display()))?;
    }
    Ok(())
}

fn remove_log_file(path: &Path) -> Result<(), String> {
    reject_symlink(path)?;
    fs::remove_file(path)
        .map_err(|error| format!("remove activity log '{}': {error}", path.display()))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn entry(
        ts: u64,
        log_type: ActivityLogType,
        level: ActivityLogLevel,
        message: &str,
    ) -> ActivityLogEntry {
        ActivityLogEntry {
            schema: ACTIVITY_LOG_SCHEMA,
            id: generated_named_id("event").unwrap(),
            ts,
            level,
            log_type,
            event: format!("{}.test", log_type.as_str()),
            message: message.to_string(),
            run_id: None,
            context: ActivityLogContext::default(),
        }
    }

    fn timestamp(day: &str, hour: u32) -> u64 {
        NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(hour, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis() as u64
    }

    #[test]
    fn appends_jsonl_and_queries_structured_filters() {
        let temp = tempfile::tempdir().unwrap();
        let store = ActivityLogStore::from_home_dir(temp.path()).unwrap();
        store
            .append(&entry(
                timestamp("2026-07-19", 10),
                ActivityLogType::Push,
                ActivityLogLevel::Success,
                "skill synced",
            ))
            .unwrap();
        store
            .append(&entry(
                timestamp("2026-07-19", 11),
                ActivityLogType::Pull,
                ActivityLogLevel::Error,
                "pull failed",
            ))
            .unwrap();

        let page = store
            .query(&ActivityLogQuery {
                types: vec![ActivityLogType::Pull],
                levels: vec![ActivityLogLevel::Error],
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].message, "pull failed");
        assert_eq!(store.stats().unwrap().file_count, 1);
    }

    #[test]
    fn query_paginates_without_repeating_entries() {
        let temp = tempfile::tempdir().unwrap();
        let store = ActivityLogStore::from_home_dir(temp.path()).unwrap();
        for hour in 1..=3 {
            store
                .append(&entry(
                    timestamp("2026-07-19", hour),
                    ActivityLogType::Push,
                    ActivityLogLevel::Info,
                    &format!("entry {hour}"),
                ))
                .unwrap();
        }
        let first = store
            .query(&ActivityLogQuery {
                limit: Some(2),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(first.entries.len(), 2);
        assert!(first.next_cursor.is_some());
        let second = store
            .query(&ActivityLogQuery {
                cursor: first.next_cursor,
                limit: Some(2),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(second.entries.len(), 1);
        assert_ne!(first.entries[0].id, second.entries[0].id);
    }

    #[test]
    fn cleanup_removes_old_segments_but_keeps_the_active_file() {
        let temp = tempfile::tempdir().unwrap();
        let store = ActivityLogStore::from_home_dir(temp.path()).unwrap();
        store
            .append(&entry(
                timestamp("2026-06-01", 1),
                ActivityLogType::System,
                ActivityLogLevel::Info,
                "old",
            ))
            .unwrap();
        store
            .append(&entry(
                timestamp("2026-07-20", 1),
                ActivityLogType::System,
                ActivityLogLevel::Info,
                "active",
            ))
            .unwrap();
        let result = store
            .cleanup_at(
                &ActivityLogCleanupRequest {
                    older_than_days: Some(30),
                    delete_all: false,
                },
                timestamp("2026-07-20", 12),
            )
            .unwrap();
        assert_eq!(result.removed_files, 1);
        assert_eq!(result.stats.file_count, 1);
    }

    #[test]
    fn rotates_full_daily_segments() {
        let temp = tempfile::tempdir().unwrap();
        let store = ActivityLogStore::from_home_dir(temp.path()).unwrap();
        let first = store.root().join("activity-2026-07-20-000.jsonl");
        File::create(&first)
            .unwrap()
            .set_len(MAX_SEGMENT_BYTES)
            .unwrap();
        store
            .append(&entry(
                timestamp("2026-07-20", 2),
                ActivityLogType::Push,
                ActivityLogLevel::Info,
                "next segment",
            ))
            .unwrap();
        assert!(store.root().join("activity-2026-07-20-001.jsonl").is_file());
    }

    #[test]
    fn cleanup_enforces_the_total_size_cap_oldest_first() {
        let temp = tempfile::tempdir().unwrap();
        let store = ActivityLogStore::from_home_dir(temp.path()).unwrap();
        for day in ["2026-07-19", "2026-07-20"] {
            store
                .append(&entry(
                    timestamp(day, 1),
                    ActivityLogType::System,
                    ActivityLogLevel::Info,
                    day,
                ))
                .unwrap();
            OpenOptions::new()
                .write(true)
                .open(store.root().join(format!("activity-{day}-000.jsonl")))
                .unwrap()
                .set_len(700 * 1024)
                .unwrap();
        }
        store
            .save_policy(&ActivityLogPolicy {
                schema: 1,
                retention_days: 30,
                max_total_bytes: 1024 * 1024,
            })
            .unwrap();
        let result = store
            .cleanup_at(
                &ActivityLogCleanupRequest::default(),
                timestamp("2026-07-20", 12),
            )
            .unwrap();
        assert_eq!(result.removed_files, 1);
        assert!(result.stats.total_bytes <= 1024 * 1024);
        assert!(store.root().join("activity-2026-07-20-000.jsonl").is_file());
    }

    #[test]
    fn concurrent_appends_produce_complete_json_lines() {
        let temp = tempfile::tempdir().unwrap();
        let store = ActivityLogStore::from_home_dir(temp.path()).unwrap();
        let mut workers = Vec::new();
        for index in 0..24 {
            let worker = store.clone();
            workers.push(std::thread::spawn(move || {
                worker
                    .append(&entry(
                        timestamp("2026-07-20", 1),
                        ActivityLogType::System,
                        ActivityLogLevel::Info,
                        &format!("event {index}"),
                    ))
                    .unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        let page = store
            .query(&ActivityLogQuery {
                limit: Some(100),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.entries.len(), 24);
    }

    #[test]
    fn delete_all_preserves_policy() {
        let temp = tempfile::tempdir().unwrap();
        let store = ActivityLogStore::from_home_dir(temp.path()).unwrap();
        let policy = ActivityLogPolicy {
            schema: 1,
            retention_days: 90,
            max_total_bytes: 25 * 1024 * 1024,
        };
        store.save_policy(&policy).unwrap();
        store
            .append(&entry(
                timestamp("2026-07-20", 1),
                ActivityLogType::System,
                ActivityLogLevel::Info,
                "entry",
            ))
            .unwrap();
        store
            .cleanup(&ActivityLogCleanupRequest {
                delete_all: true,
                older_than_days: None,
            })
            .unwrap();
        assert_eq!(store.stats().unwrap().file_count, 0);
        assert_eq!(store.load_policy().unwrap(), policy);
    }

    #[test]
    fn sensitive_values_are_redacted_before_persistence() {
        let temp = tempfile::tempdir().unwrap();
        let store = ActivityLogStore::from_home_dir(temp.path()).unwrap();
        let item = ActivityLogEntry::new(
            ActivityLogType::Storage,
            "error",
            "storage.failed",
            "Authorization: Bearer secret-token x-amz-signature=abc123",
            None,
            ActivityLogContext::default(),
        );
        store.append(&item).unwrap();
        let file = store.log_files().unwrap().remove(0).path;
        let mut body = String::new();
        File::open(file).unwrap().read_to_string(&mut body).unwrap();
        assert!(!body.contains("secret-token"));
        assert!(!body.contains("abc123"));
        assert!(body.contains("[redacted]"));
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_rejects_log_named_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let store = ActivityLogStore::from_home_dir(temp.path()).unwrap();
        symlink(
            outside.path(),
            store.root().join("activity-2026-07-20-000.jsonl"),
        )
        .unwrap();
        assert!(store
            .cleanup(&ActivityLogCleanupRequest::default())
            .is_err());
    }
}
