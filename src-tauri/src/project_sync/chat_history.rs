//! Local-only Git-to-Codex history discovery for a registered project.
//!
//! This module deliberately does not participate in bundle capture. Titles,
//! first-message summaries, and inferred Git relationships stay on this
//! machine and are recomputed from the active project binding on demand.

use super::domain::{BindingState, LocalProjectId, MaterializationStatus, Provider};
use super::persistence::{read_json_bounded, write_json_atomic, LocalStateRepository};
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const DEFAULT_COMMIT_LIMIT: usize = 50;
const MAX_COMMIT_LIMIT: usize = 50;
const MAX_GIT_COMMITS: usize = 10_000;
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SUMMARY_CHARS: usize = 180;
const ACTIVE_WINDOW_SECS: u64 = 5 * 60;
const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_ROLLOUT_IDENTITY_RECORDS: usize = 16;
const DEFAULT_HISTORY_WINDOW_DAYS: u64 = 30;
const DAY_SECS: u64 = 24 * 60 * 60;
const CHAT_CACHE_SCHEMA: u32 = 1;
const MAX_CHAT_CACHE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CHAT_CACHE_ENTRIES: usize = 50_000;
#[cfg(target_os = "macos")]
const CODEX_PROJECT_ROUTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(target_os = "macos")]
const CODEX_PROJECT_ROUTE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CodexThreadSummary {
    pub thread_id: String,
    #[serde(default)]
    pub working_directory: String,
    pub title: String,
    pub summary: String,
    pub started_at: u64,
    pub ended_at: u64,
    pub branch: Option<String>,
    pub recorded_sha: Option<String>,
    pub is_active: bool,
    #[serde(default)]
    pub user_round_count: usize,
    #[serde(default)]
    pub agent_message_count: usize,
    #[serde(default)]
    pub tool_call_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub metrics_complete: bool,
    #[serde(default)]
    pub commit_occurrence_count: usize,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatTurnRole {
    User,
    Assistant,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ChatTurnPreview {
    pub ordinal: usize,
    pub role: ChatTurnRole,
    pub timestamp: Option<u64>,
    pub preview: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CodexThreadDetailsPage {
    pub thread_id: String,
    pub turns: Vec<ChatTurnPreview>,
    pub next_cursor: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CommitThreadReference {
    pub thread_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GitCommitSummary {
    pub sha: String,
    pub short_sha: String,
    pub committed_at: u64,
    pub subject: String,
    pub thread_refs: Vec<CommitThreadReference>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GitBranchSummary {
    pub name: String,
    pub is_current: bool,
    pub available: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GitHistoryPage {
    pub selected_branch: String,
    pub branches: Vec<GitBranchSummary>,
    pub commits: Vec<GitCommitSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_repository_url: Option<String>,
    pub next_cursor: Option<String>,
    pub unique_thread_count: usize,
    pub reference_count: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UnmappedThreadReference {
    pub thread_id: String,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectChatHistory {
    pub project_id: String,
    pub codex_home: String,
    pub threads: Vec<CodexThreadSummary>,
    pub git: Option<GitHistoryPage>,
    pub unmapped: Vec<UnmappedThreadReference>,
    pub warnings: Vec<String>,
    pub window_start: u64,
    pub window_end: u64,
    pub next_before: Option<u64>,
    pub storage_sync: Vec<StorageSyncSummary>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageSyncSummary {
    pub storage_id: String,
    pub storage_name: String,
    pub last_pull_at: Option<u64>,
    pub last_push_at: Option<u64>,
}

#[derive(Clone, Debug)]
struct SessionIndexEntry {
    title: Option<String>,
    updated_at: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ParsedRollout {
    thread: CodexThreadSummary,
    cwd: PathBuf,
    #[serde(default)]
    has_record_endpoints: bool,
    #[serde(default)]
    is_internal_subagent: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct RolloutIdentity {
    #[serde(default)]
    thread_id: Option<String>,
    cwd: PathBuf,
    is_internal_subagent: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct RolloutIdentityCacheEntry {
    size: u64,
    modified_nanos: u64,
    identity: RolloutIdentity,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct RolloutCacheEntry {
    size: u64,
    modified_nanos: u64,
    parsed: ParsedRollout,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ChatHistoryCache {
    schema: u32,
    #[serde(default)]
    entries: BTreeMap<String, RolloutCacheEntry>,
    #[serde(default)]
    ownership: BTreeMap<String, RolloutIdentityCacheEntry>,
}

impl Default for ChatHistoryCache {
    fn default() -> Self {
        Self {
            schema: CHAT_CACHE_SCHEMA,
            entries: BTreeMap::new(),
            ownership: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
struct MappingResult {
    unmapped: Vec<UnmappedThreadReference>,
    unique_thread_count: usize,
    reference_count: usize,
}

#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
struct GitDiscovery {
    selected_branch: String,
    selected_ref_available: bool,
    branches: Vec<GitBranchSummary>,
    commits: Vec<GitCommitSummary>,
    next_cursor: Option<String>,
}

struct ResolvedProject {
    project_id: LocalProjectId,
    project_root: PathBuf,
    codex_home: PathBuf,
}

pub fn list_project_repository_kinds(
    repository: &LocalStateRepository,
) -> Result<BTreeMap<String, bool>, String> {
    let config = repository.load_config()?;
    let bindings = repository.load_bindings()?;
    Ok(config
        .projects
        .iter()
        .filter_map(|project| {
            let root = bindings
                .bindings
                .iter()
                .find(|binding| {
                    binding.local_project_id == project.local_project_id
                        && binding.state == BindingState::Active
                })
                .and_then(|binding| fs::canonicalize(&binding.project_root).ok())?;
            let is_git =
                git_output(&root, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|output| {
                    output.status.success()
                        && String::from_utf8_lossy(&output.stdout).trim() == "true"
                });
            Some((project.local_project_id.to_string(), is_git))
        })
        .collect())
}

/// Recompute a project's local Codex history and its best-effort relationship
/// to the selected Git branch. The result contains no synced metadata.
pub fn get_project_chat_history(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    branch: Option<&str>,
    before_time: Option<u64>,
    window_days: Option<u64>,
    force_revalidate: bool,
) -> Result<ProjectChatHistory, String> {
    let resolved = resolve_project(repository, local_project_id)?;
    let mut warnings = Vec::new();
    let mut all_threads = scan_codex_threads(
        repository,
        &resolved.codex_home,
        &resolved.project_root,
        force_revalidate,
        &mut warnings,
    )?;
    all_threads.sort_by(|left, right| {
        right
            .ended_at
            .cmp(&left.ended_at)
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    let window_end = before_time.unwrap_or_else(|| now_secs().saturating_add(1));
    let window_days = window_days
        .unwrap_or(DEFAULT_HISTORY_WINDOW_DAYS)
        .clamp(1, 90);
    let window_start = window_end.saturating_sub(window_days.saturating_mul(DAY_SECS));
    let mut threads = all_threads
        .iter()
        .filter(|thread| thread_in_window(thread, window_start, window_end))
        .cloned()
        .collect::<Vec<_>>();

    let recorded_branches = all_threads
        .iter()
        .filter_map(|thread| thread.branch.as_ref())
        .filter(|branch| !branch.trim().is_empty())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut git = discover_git_history_with_recorded_branches(
        &resolved.project_root,
        branch,
        None,
        DEFAULT_COMMIT_LIMIT,
        &recorded_branches,
    )?;
    let mut unmapped = Vec::new();

    let git_page = if let Some(discovery) = git.as_mut() {
        let github_repository_url = discover_github_repository_url(&resolved.project_root);
        // Mapping must use the complete bounded first-parent rail, rather than
        // just the visible page, so pagination does not change classification.
        let all_commits = if discovery.selected_ref_available {
            load_first_parent_commits(
                &resolved.project_root,
                &discovery.selected_branch,
                MAX_GIT_COMMITS + 1,
            )?
        } else {
            Vec::new()
        };
        let was_truncated = all_commits.len() > MAX_GIT_COMMITS;
        let mut all_commits = all_commits
            .into_iter()
            .take(MAX_GIT_COMMITS)
            .collect::<Vec<_>>();
        if was_truncated {
            warnings.push(format!(
                "Git history was limited to the newest {MAX_GIT_COMMITS} first-parent commits"
            ));
        }
        let selected_branch = discovery.selected_branch.clone();
        let mapping = map_threads_to_commits(&threads, &mut all_commits, &selected_branch);
        unmapped = mapping.unmapped;

        let occurrences = all_commits
            .iter()
            .flat_map(|commit| commit.thread_refs.iter())
            .fold(BTreeMap::<String, usize>::new(), |mut counts, reference| {
                *counts.entry(reference.thread_id.clone()).or_default() += 1;
                counts
            });
        for thread in &mut threads {
            thread.commit_occurrence_count =
                occurrences.get(&thread.thread_id).copied().unwrap_or(0);
        }

        let visible_commits = all_commits
            .iter()
            .filter(|commit| {
                (commit.committed_at >= window_start && commit.committed_at < window_end)
                    || !commit.thread_refs.is_empty()
            })
            .cloned()
            .collect::<Vec<_>>();
        Some(GitHistoryPage {
            selected_branch: discovery.selected_branch.clone(),
            branches: discovery.branches.clone(),
            commits: visible_commits,
            github_repository_url,
            next_cursor: None,
            unique_thread_count: mapping.unique_thread_count,
            reference_count: mapping.reference_count,
        })
    } else {
        None
    };

    let has_older_threads = all_threads
        .iter()
        .any(|thread| thread.ended_at < window_start);
    let has_older_commits = git_page.as_ref().is_some_and(|_| {
        git.as_ref()
            .is_some_and(|discovery| discovery.selected_ref_available)
            && load_first_parent_commits(
                &resolved.project_root,
                git.as_ref()
                    .map(|item| item.selected_branch.as_str())
                    .unwrap_or("HEAD"),
                MAX_GIT_COMMITS + 1,
            )
            .is_ok_and(|commits| {
                commits
                    .iter()
                    .any(|commit| commit.committed_at < window_start)
            })
    });

    Ok(ProjectChatHistory {
        project_id: resolved.project_id.to_string(),
        codex_home: resolved.codex_home.to_string_lossy().to_string(),
        threads,
        git: git_page,
        unmapped,
        warnings,
        window_start,
        window_end,
        next_before: (has_older_threads || has_older_commits).then_some(window_start),
        storage_sync: project_storage_sync(repository, local_project_id)?,
    })
}

fn thread_in_window(thread: &CodexThreadSummary, window_start: u64, window_end: u64) -> bool {
    thread.ended_at >= window_start && thread.ended_at < window_end
}

fn project_storage_sync(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<Vec<StorageSyncSummary>, String> {
    let config = repository.load_config()?;
    let project = config
        .project(local_project_id)
        .ok_or_else(|| "project is not registered".to_string())?;
    let materializations = repository.load_materializations()?;
    let mut result = Vec::new();
    for link in config
        .links
        .iter()
        .filter(|link| &link.local_project_id == local_project_id)
    {
        let storage_name = config
            .storages
            .iter()
            .find(|storage| storage.id == link.storage_id)
            .map(|storage| storage.name.clone())
            .unwrap_or_else(|| link.storage_id.to_string());
        let base = project.recipe_bases.get(&link.storage_id);
        let historical_pull = materializations
            .records
            .iter()
            .filter(|record| {
                &record.local_project_id == local_project_id
                    && record.storage_id == link.storage_id
                    && record.status == MaterializationStatus::Complete
            })
            .map(|record| record.applied_at)
            .max();
        result.push(StorageSyncSummary {
            storage_id: link.storage_id.to_string(),
            storage_name,
            last_pull_at: base.and_then(|base| base.last_pull_at).or(historical_pull),
            last_push_at: base.and_then(|base| base.last_push_at),
        });
    }
    result.sort_by(|left, right| left.storage_name.cmp(&right.storage_name));
    Ok(result)
}

pub fn get_project_chat_thread_details(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    thread_id: &str,
    cursor: Option<usize>,
    limit: Option<usize>,
) -> Result<CodexThreadDetailsPage, String> {
    let (_, rollout_path) = resolve_owned_rollout(repository, local_project_id, thread_id)?;
    let mut page = parse_thread_detail_file(&rollout_path, cursor, limit.unwrap_or(10))?;
    page.thread_id = thread_id.to_string();
    Ok(page)
}

/// Revalidates both project registration and rollout ownership before opening
/// an argument-safe Codex resume command in macOS Terminal.
pub fn open_codex_thread_in_terminal(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    thread_id: &str,
    before_launch: impl FnOnce(&str),
) -> Result<(), String> {
    let resolved = resolve_owned_thread(repository, local_project_id, thread_id)?;
    launch_terminal_resume(
        thread_id,
        &resolved.project_root,
        &resolved.codex_home,
        before_launch,
    )
}

pub fn open_codex_thread_in_app(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    thread_id: &str,
    before_launch: impl FnMut(&str),
) -> Result<(), String> {
    let resolved = resolve_owned_thread(repository, local_project_id, thread_id)?;
    launch_codex_app(
        thread_id,
        &resolved.project_root,
        &resolved.codex_home,
        before_launch,
    )
}

/// Fast launch guard used by the desktop deep-link action. It deliberately
/// skips Git discovery/mapping while enforcing the same registration,
/// binding, profile, UUID, and rollout-ownership checks as Terminal launch.
pub fn validate_codex_thread_ownership(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    thread_id: &str,
) -> Result<(), String> {
    resolve_owned_thread(repository, local_project_id, thread_id).map(|_| ())
}

fn resolve_owned_thread(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    thread_id: &str,
) -> Result<ResolvedProject, String> {
    validate_thread_uuid(thread_id)?;
    let resolved = resolve_project(repository, local_project_id)?;
    let mut warnings = Vec::new();
    let owned = scan_codex_threads(
        repository,
        &resolved.codex_home,
        &resolved.project_root,
        false,
        &mut warnings,
    )?
    .into_iter()
    .any(|thread| thread.thread_id == thread_id);
    if !owned {
        return Err("Codex thread does not belong to the selected project".to_string());
    }
    Ok(resolved)
}

fn resolve_owned_rollout(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    thread_id: &str,
) -> Result<(ResolvedProject, PathBuf), String> {
    validate_thread_uuid(thread_id)?;
    let resolved = resolve_project(repository, local_project_id)?;
    let rollout_path = find_owned_rollout(&resolved.codex_home, &resolved.project_root, thread_id)?;
    Ok((resolved, rollout_path))
}

fn find_owned_rollout(
    codex_home: &Path,
    project_root: &Path,
    thread_id: &str,
) -> Result<PathBuf, String> {
    let mut rollout_paths = Vec::new();
    let mut named_candidates = Vec::new();
    for directory_name in ["sessions", "archived_sessions"] {
        let directory = codex_home.join(directory_name);
        if !directory.exists() {
            continue;
        }
        for entry in WalkDir::new(&directory)
            .follow_links(false)
            .max_depth(8)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            let path = entry.path().to_path_buf();
            if rollout_filename_matches_thread(&path, thread_id) {
                named_candidates.push(path.clone());
            }
            rollout_paths.push(path);
        }
    }

    let mut best: Option<(ParsedRollout, PathBuf)> = None;
    for path in &named_candidates {
        consider_owned_rollout(path, project_root, thread_id, &mut best)?;
    }

    // Current Codex rollouts end with their UUID, so the normal path parses
    // only the requested transcript. If an older or renamed rollout does not,
    // inspect only its bounded session metadata before deciding to parse it.
    if best.is_none() {
        for path in rollout_paths
            .iter()
            .filter(|path| !rollout_filename_matches_thread(path, thread_id))
        {
            let identity = match read_rollout_identity(path) {
                Ok(Some(identity)) => identity,
                Ok(None) | Err(_) => continue,
            };
            if identity.thread_id.as_deref() != Some(thread_id) {
                continue;
            }
            consider_owned_rollout(path, project_root, thread_id, &mut best)?;
        }
    }

    best.map(|(_, path)| path)
        .ok_or_else(|| "Codex thread does not belong to the selected project".to_string())
}

fn rollout_filename_matches_thread(path: &Path, thread_id: &str) -> bool {
    path.file_stem()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.ends_with(thread_id))
}

fn consider_owned_rollout(
    path: &Path,
    project_root: &Path,
    thread_id: &str,
    best: &mut Option<(ParsedRollout, PathBuf)>,
) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("read '{}': {error}", path.display()))?;
    let mut warnings = Vec::new();
    let Ok(parsed) = parse_rollout_file(path, modified_secs(&metadata).unwrap_or(0), &mut warnings)
    else {
        return Ok(());
    };
    if parsed.thread.thread_id != thread_id || !cwd_belongs_to_project(&parsed.cwd, project_root) {
        return Ok(());
    }
    let replace = best
        .as_ref()
        .is_none_or(|(current, _)| rollout_is_preferred(&parsed, current));
    if replace {
        *best = Some((parsed, path.to_path_buf()));
    }
    Ok(())
}

fn rollout_is_preferred(candidate: &ParsedRollout, current: &ParsedRollout) -> bool {
    candidate
        .thread
        .metrics_complete
        .cmp(&current.thread.metrics_complete)
        .then_with(|| candidate.thread.ended_at.cmp(&current.thread.ended_at))
        .then_with(|| {
            candidate
                .has_record_endpoints
                .cmp(&current.has_record_endpoints)
        })
        .is_gt()
}

fn resolve_project(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<ResolvedProject, String> {
    let config = repository.load_config()?;
    if config.project(local_project_id).is_none() {
        return Err(
            "project is not registered; unfinished setup drafts have no history".to_string(),
        );
    }
    let state = repository.load_bindings()?;
    let binding = state
        .bindings
        .iter()
        .find(|binding| {
            &binding.local_project_id == local_project_id && binding.state == BindingState::Active
        })
        .ok_or_else(|| "project has no active binding on this machine".to_string())?;

    let project_root = fs::canonicalize(&binding.project_root)
        .map_err(|error| format!("resolve project root '{}': {error}", binding.project_root))?;
    if project_root != PathBuf::from(&binding.canonical_project_root) {
        return Err(
            "project root changed since the binding was saved; open Project Settings".to_string(),
        );
    }
    let profile_id = binding
        .profile_ids
        .get(&Provider::Codex)
        .ok_or_else(|| "project has no Codex profile; open Project Settings".to_string())?;
    let profile = state
        .profiles
        .iter()
        .find(|profile| &profile.profile_id == profile_id && profile.provider == Provider::Codex)
        .ok_or_else(|| "project's Codex profile is missing; open Project Settings".to_string())?;
    let codex_home = fs::canonicalize(&profile.path)
        .map_err(|error| format!("resolve Codex profile '{}': {error}", profile.path))?;
    if codex_home != PathBuf::from(&profile.canonical_path) {
        return Err("Codex profile path changed; open Project Settings".to_string());
    }

    Ok(ResolvedProject {
        project_id: local_project_id.clone(),
        project_root,
        codex_home,
    })
}

fn scan_codex_threads(
    repository: &LocalStateRepository,
    codex_home: &Path,
    project_root: &Path,
    force_revalidate: bool,
    warnings: &mut Vec<String>,
) -> Result<Vec<CodexThreadSummary>, String> {
    let index_path = codex_home.join("session_index.jsonl");
    let index = read_session_index(&index_path, warnings)?;
    let now = now_secs();
    let mut by_id = BTreeMap::<String, (CodexThreadSummary, bool)>::new();
    let cache_path = repository.root().join("chat_history_cache.json");
    let mut cache = load_chat_history_cache(repository.root(), &cache_path, warnings);
    let profile_cache_prefix = format!("{}\u{0}", codex_home.display());
    let mut seen_cache_keys = BTreeSet::new();

    for directory_name in ["sessions", "archived_sessions"] {
        let directory = codex_home.join(directory_name);
        if !directory.exists() {
            continue;
        }
        for entry in WalkDir::new(&directory)
            .follow_links(false)
            .max_depth(8)
            .into_iter()
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    warnings.push(format!("Cannot inspect a Codex session path: {error}"));
                    continue;
                }
            };
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    warnings.push(format!("Cannot read '{}': {error}", entry.path().display()));
                    continue;
                }
            };
            let fallback_time = modified_secs(&metadata).unwrap_or(0);
            let cache_key = format!("{}\u{0}{}", codex_home.display(), entry.path().display());
            seen_cache_keys.insert(cache_key.clone());
            let modified_nanos = modified_nanos(&metadata).unwrap_or(0);
            let cached_identity = cache
                .ownership
                .get(&cache_key)
                .filter(|cached| {
                    cached.size == metadata.len() && cached.modified_nanos == modified_nanos
                })
                .map(|cached| cached.identity.clone())
                .or_else(|| {
                    cache
                        .entries
                        .get(&cache_key)
                        .filter(|cached| {
                            cached.size == metadata.len() && cached.modified_nanos == modified_nanos
                        })
                        .map(|cached| RolloutIdentity {
                            thread_id: Some(cached.parsed.thread.thread_id.clone()),
                            cwd: cached.parsed.cwd.clone(),
                            is_internal_subagent: cached.parsed.is_internal_subagent,
                        })
                });
            let belongs_to_project = match cached_identity {
                Some(identity) => {
                    !identity.is_internal_subagent
                        && cwd_belongs_to_project(&identity.cwd, project_root)
                }
                None => match read_rollout_identity(entry.path()) {
                    Ok(Some(identity)) => {
                        let belongs = !identity.is_internal_subagent
                            && cwd_belongs_to_project(&identity.cwd, project_root);
                        cache.ownership.insert(
                            cache_key.clone(),
                            RolloutIdentityCacheEntry {
                                size: metadata.len(),
                                modified_nanos,
                                identity,
                            },
                        );
                        belongs
                    }
                    // Legacy or malformed rollouts still go through the full parser so the
                    // existing compatibility and warning behavior remains intact.
                    Ok(None) | Err(_) => true,
                },
            };
            if !belongs_to_project {
                continue;
            }
            let parsed = if !force_revalidate {
                cache.entries.get(&cache_key).and_then(|cached| {
                    cached_rollout(
                        cached,
                        metadata.len(),
                        modified_nanos,
                        index
                            .get(&cached.parsed.thread.thread_id)
                            .and_then(|item| item.title.as_ref())
                            .is_some(),
                    )
                })
            } else {
                None
            };
            let parsed = match parsed {
                Some(parsed) => parsed,
                None => match parse_rollout_file(entry.path(), fallback_time, warnings) {
                    Ok(parsed) => {
                        cache.ownership.insert(
                            cache_key.clone(),
                            RolloutIdentityCacheEntry {
                                size: metadata.len(),
                                modified_nanos,
                                identity: RolloutIdentity {
                                    thread_id: Some(parsed.thread.thread_id.clone()),
                                    cwd: parsed.cwd.clone(),
                                    is_internal_subagent: parsed.is_internal_subagent,
                                },
                            },
                        );
                        cache.entries.insert(
                            cache_key,
                            RolloutCacheEntry {
                                size: metadata.len(),
                                modified_nanos,
                                parsed: cache_safe_rollout(parsed.clone()),
                            },
                        );
                        parsed
                    }
                    Err(error) => {
                        warnings.push(format!("Skipped '{}': {error}", entry.path().display()));
                        continue;
                    }
                },
            };
            if !rollout_is_user_visible(&parsed) {
                continue;
            }
            if !cwd_belongs_to_project(&parsed.cwd, project_root) {
                continue;
            }
            let has_record_endpoints = parsed.has_record_endpoints;
            let working_directory = parsed.cwd.to_string_lossy().into_owned();
            let mut thread = parsed.thread;
            if thread.working_directory.is_empty() {
                thread.working_directory = working_directory;
            }
            if let Some(index_entry) = index.get(&thread.thread_id) {
                apply_index_metadata(&mut thread, has_record_endpoints, index_entry);
            }
            if thread.title.is_empty() {
                thread.title = if thread.summary.is_empty() {
                    format!("Codex thread {}", short_id(&thread.thread_id))
                } else {
                    thread.summary.clone()
                };
            }
            thread.is_active = session_mtime_is_active(now, fallback_time);
            if fallback_time > now.saturating_add(ACTIVE_WINDOW_SECS) {
                warnings.push(format!(
                    "Codex rollout '{}' has a modified time in the future",
                    entry.path().display()
                ));
            }
            let replace =
                by_id
                    .get(&thread.thread_id)
                    .is_none_or(|(previous, previous_endpoints)| {
                        thread
                            .metrics_complete
                            .cmp(&previous.metrics_complete)
                            .then_with(|| thread.ended_at.cmp(&previous.ended_at))
                            .then_with(|| has_record_endpoints.cmp(previous_endpoints))
                            .is_gt()
                    });
            if replace {
                by_id.insert(thread.thread_id.clone(), (thread, has_record_endpoints));
            }
        }
    }
    cache
        .entries
        .retain(|key, _| !key.starts_with(&profile_cache_prefix) || seen_cache_keys.contains(key));
    cache
        .ownership
        .retain(|key, _| !key.starts_with(&profile_cache_prefix) || seen_cache_keys.contains(key));
    if cache.entries.len() > MAX_CHAT_CACHE_ENTRIES
        || cache.ownership.len() > MAX_CHAT_CACHE_ENTRIES
    {
        warnings.push(format!(
            "Local chat history cache exceeded {MAX_CHAT_CACHE_ENTRIES} entries and was reset"
        ));
        cache = ChatHistoryCache::default();
    }
    save_chat_history_cache(repository.root(), &cache_path, &cache, warnings);
    Ok(by_id.into_values().map(|(thread, _)| thread).collect())
}

fn apply_index_metadata(
    thread: &mut CodexThreadSummary,
    has_record_endpoints: bool,
    index_entry: &SessionIndexEntry,
) {
    if let Some(title) = &index_entry.title {
        thread.title = title.clone();
    }
    if !has_record_endpoints {
        if let Some(updated) = index_entry.updated_at {
            thread.ended_at = updated.max(thread.started_at);
        }
    }
}

fn load_chat_history_cache(
    root: &Path,
    path: &Path,
    warnings: &mut Vec<String>,
) -> ChatHistoryCache {
    match read_json_bounded::<ChatHistoryCache>(root, path, MAX_CHAT_CACHE_BYTES) {
        Ok(Some(cache))
            if cache.schema == CHAT_CACHE_SCHEMA
                && cache.entries.len() <= MAX_CHAT_CACHE_ENTRIES
                && cache.ownership.len() <= MAX_CHAT_CACHE_ENTRIES =>
        {
            cache
        }
        Ok(Some(_)) => {
            warnings.push(format!(
                "Ignored incompatible or oversized local chat history cache '{}'",
                path.display()
            ));
            ChatHistoryCache::default()
        }
        Ok(None) => ChatHistoryCache::default(),
        Err(error) => {
            warnings.push(format!(
                "Ignored unreadable local chat history cache '{}': {error}",
                path.display()
            ));
            ChatHistoryCache::default()
        }
    }
}

fn cached_rollout(
    entry: &RolloutCacheEntry,
    size: u64,
    modified_nanos: u64,
    has_indexed_title: bool,
) -> Option<ParsedRollout> {
    (entry.size == size && entry.modified_nanos == modified_nanos && has_indexed_title)
        .then(|| entry.parsed.clone())
}

fn cache_safe_rollout(mut parsed: ParsedRollout) -> ParsedRollout {
    parsed.thread.title.clear();
    parsed.thread.summary.clear();
    parsed
}

fn save_chat_history_cache(
    root: &Path,
    path: &Path,
    cache: &ChatHistoryCache,
    warnings: &mut Vec<String>,
) {
    if let Err(error) = write_json_atomic(root, path, cache, MAX_CHAT_CACHE_BYTES) {
        warnings.push(format!(
            "Could not update local chat history cache '{}': {error}",
            path.display()
        ));
    }
}

fn read_session_index(
    path: &Path,
    warnings: &mut Vec<String>,
) -> Result<BTreeMap<String, SessionIndexEntry>, String> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let metadata = fs::metadata(path)
        .map_err(|error| format!("read Codex session index '{}': {error}", path.display()))?;
    if metadata.len() > MAX_INDEX_BYTES {
        warnings.push(format!(
            "Codex session index exceeded {} MiB and was ignored",
            MAX_INDEX_BYTES / (1024 * 1024)
        ));
        return Ok(BTreeMap::new());
    }
    let lines = read_bounded_lines(path, usize::MAX, warnings)?;
    Ok(parse_session_index_lines(lines.iter().map(String::as_str)))
}

fn parse_session_index_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, SessionIndexEntry> {
    let mut result = BTreeMap::<String, SessionIndexEntry>::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(id) = string_alias(&value, &["id", "thread_id", "session_id"]) else {
            continue;
        };
        let entry = result.entry(id.to_string()).or_insert(SessionIndexEntry {
            title: None,
            updated_at: None,
        });
        if let Some(title) = string_alias(&value, &["thread_name", "title", "name"])
            .map(normalize_summary)
            .map(|title| truncate_chars(&title, MAX_SUMMARY_CHARS))
            .filter(|title| !title.is_empty())
        {
            entry.title = Some(title);
        }
        if let Some(updated) = value_alias(&value, &["updated_at", "updatedAt", "last_updated_at"])
            .and_then(parse_timestamp_value)
        {
            entry.updated_at = Some(updated);
        }
    }
    result
}

pub(crate) fn parse_session_index_titles(bytes: &[u8]) -> BTreeMap<String, String> {
    parse_session_index_lines(
        bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| line.len() <= MAX_LINE_BYTES)
            .filter_map(|line| std::str::from_utf8(line).ok()),
    )
    .into_iter()
    .filter_map(|(thread_id, entry)| entry.title.map(|title| (thread_id, title)))
    .collect()
}

pub(crate) fn first_codex_user_summary(bytes: &[u8]) -> Option<String> {
    let mut parser = RolloutParser::new(0);
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty() && line.len() <= MAX_LINE_BYTES)
    {
        if let Ok(value) = serde_json::from_slice::<Value>(line) {
            parser.consume(&value);
            if parser.summary.is_some() {
                break;
            }
        }
    }
    parser.summary.or(parser.fallback_summary)
}

#[cfg(test)]
fn parse_rollout_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    fallback_time: u64,
) -> Result<ParsedRollout, String> {
    let mut parser = RolloutParser::new(fallback_time);
    for line in lines {
        match serde_json::from_str::<Value>(line) {
            Ok(value) => parser.consume(&value),
            Err(_) => parser.metrics_complete = false,
        }
    }
    parser.finish()
}

fn parse_rollout_file(
    path: &Path,
    fallback_time: u64,
    warnings: &mut Vec<String>,
) -> Result<ParsedRollout, String> {
    let file = File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut parser = RolloutParser::new(fallback_time);
    let mut bytes = Vec::new();
    let mut malformed_oversized_warned = false;
    loop {
        let record_start = reader
            .stream_position()
            .map_err(|error| format!("read '{}': {error}", path.display()))?;
        match read_bounded_line(&mut reader, &mut bytes, MAX_LINE_BYTES)
            .map_err(|error| format!("read '{}': {error}", path.display()))?
        {
            BoundedLine::Eof => break,
            BoundedLine::Oversized => {
                let record_end = reader
                    .stream_position()
                    .map_err(|error| format!("read '{}': {error}", path.display()))?;
                match recover_oversized_record(&mut reader, record_start, record_end) {
                    Ok(value) => parser.consume(&value),
                    Err(error) => {
                        parser.metrics_complete = false;
                        if !malformed_oversized_warned {
                            warnings.push(format!(
                                "Ignored malformed oversized JSONL record in '{}': {error}",
                                path.display()
                            ));
                            malformed_oversized_warned = true;
                        }
                    }
                }
                continue;
            }
            BoundedLine::Line => {}
        }
        if bytes.is_empty() {
            continue;
        }
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => parser.consume(&value),
            Err(error) => {
                parser.metrics_complete = false;
                warnings.push(format!(
                    "Ignored malformed JSONL record in '{}': {error}",
                    path.display()
                ));
            }
        }
    }
    parser.finish()
}

/// Read only the small rollout prefix needed to determine project ownership.
/// Codex writes `session_meta` at the start of a rollout; the bounded fallback
/// keeps an unexpected legacy file from turning this probe into another full
/// transcript scan.
fn read_rollout_identity(path: &Path) -> Result<Option<RolloutIdentity>, String> {
    let file = File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    for _ in 0..MAX_ROLLOUT_IDENTITY_RECORDS {
        match read_bounded_line(&mut reader, &mut bytes, MAX_LINE_BYTES)
            .map_err(|error| format!("read '{}': {error}", path.display()))?
        {
            BoundedLine::Eof => return Ok(None),
            BoundedLine::Oversized => continue,
            BoundedLine::Line => {}
        }
        if bytes.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if let Some(identity) = rollout_identity_from_value(&value) {
            return Ok(Some(identity));
        }
    }
    Ok(None)
}

fn rollout_identity_from_value(value: &Value) -> Option<RolloutIdentity> {
    let payload = value.get("payload").unwrap_or(value);
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let payload_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if event_type != "session_meta" && payload_type != "session_meta" {
        return None;
    }
    let cwd = string_alias(payload, &["cwd", "working_directory", "project_path"])?;
    Some(RolloutIdentity {
        thread_id: string_alias(payload, &["id", "thread_id", "session_id"]).map(ToOwned::to_owned),
        cwd: PathBuf::from(cwd),
        is_internal_subagent: session_meta_is_internal_subagent(payload),
    })
}

fn session_meta_is_internal_subagent(payload: &Value) -> bool {
    payload.get("thread_source").and_then(Value::as_str) == Some("subagent")
        || payload
            .get("source")
            .and_then(|source| source.get("subagent"))
            .is_some()
}

struct RolloutParser {
    fallback_time: u64,
    thread_id: Option<String>,
    cwd: Option<PathBuf>,
    first_timestamp: Option<u64>,
    last_timestamp: Option<u64>,
    metadata_timestamp: Option<u64>,
    branch: Option<String>,
    recorded_sha: Option<String>,
    summary: Option<String>,
    fallback_summary: Option<String>,
    user_round_count: usize,
    agent_message_count: usize,
    tool_call_count: usize,
    total_tokens: Option<u64>,
    metrics_complete: bool,
    is_internal_subagent: bool,
}

impl RolloutParser {
    fn new(fallback_time: u64) -> Self {
        Self {
            fallback_time,
            thread_id: None,
            cwd: None,
            first_timestamp: None,
            last_timestamp: None,
            metadata_timestamp: None,
            branch: None,
            recorded_sha: None,
            summary: None,
            fallback_summary: None,
            user_round_count: 0,
            agent_message_count: 0,
            tool_call_count: 0,
            total_tokens: None,
            metrics_complete: true,
            is_internal_subagent: false,
        }
    }

    fn consume(&mut self, value: &Value) {
        let payload = value.get("payload").unwrap_or(value);
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload_type = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(timestamp) = value_alias(value, &["timestamp", "created_at", "updated_at"])
            .or_else(|| value_alias(payload, &["timestamp", "created_at", "updated_at"]))
            .and_then(parse_timestamp_value)
        {
            self.first_timestamp.get_or_insert(timestamp);
            self.last_timestamp = Some(timestamp);
        }
        if event_type == "session_meta" || payload_type == "session_meta" {
            self.is_internal_subagent |= session_meta_is_internal_subagent(payload);
            if self.thread_id.is_none() {
                self.thread_id = string_alias(payload, &["id", "thread_id", "session_id"])
                    .map(ToOwned::to_owned);
            }
            if self.cwd.is_none() {
                self.cwd = string_alias(payload, &["cwd", "working_directory", "project_path"])
                    .map(PathBuf::from);
            }
            if self.metadata_timestamp.is_none() {
                self.metadata_timestamp =
                    value_alias(value, &["timestamp", "created_at", "started_at"])
                        .or_else(|| {
                            value_alias(payload, &["timestamp", "created_at", "started_at"])
                        })
                        .and_then(parse_timestamp_value);
            }
            let git = payload.get("git").unwrap_or(payload);
            if self.branch.is_none() {
                self.branch = string_alias(git, &["branch", "git_branch"])
                    .or_else(|| string_alias(payload, &["git_branch", "branch"]))
                    .map(ToOwned::to_owned);
            }
            if self.recorded_sha.is_none() {
                self.recorded_sha =
                    string_alias(git, &["commit_hash", "commit", "sha", "git_commit"])
                        .or_else(|| string_alias(payload, &["git_commit", "commit_hash"]))
                        .map(ToOwned::to_owned);
            }
        }
        if payload_type == "user_message" {
            if let Some(text) = genuine_user_event(payload) {
                self.user_round_count = self.user_round_count.saturating_add(1);
                if self.summary.is_none() {
                    self.summary = Some(truncate_chars(&text, MAX_SUMMARY_CHARS));
                }
            }
        } else if payload_type == "agent_message" {
            self.agent_message_count = self.agent_message_count.saturating_add(1);
        }
        if matches!(payload_type, "function_call" | "custom_tool_call") {
            self.tool_call_count = self.tool_call_count.saturating_add(1);
        }
        if payload_type == "token_count" {
            if let Some(total) = payload
                .get("info")
                .and_then(|info| info.get("total_token_usage"))
                .and_then(|usage| usage.get("total_tokens"))
                .and_then(Value::as_u64)
            {
                self.total_tokens = Some(self.total_tokens.unwrap_or(0).max(total));
            }
        }
        if self.fallback_summary.is_none() {
            self.fallback_summary = extract_user_message(value)
                .map(|text| truncate_chars(&normalize_summary(&text), MAX_SUMMARY_CHARS))
                .filter(|text| is_meaningful_user_text(text));
        }
    }

    fn finish(self) -> Result<ParsedRollout, String> {
        let thread_id = self
            .thread_id
            .ok_or_else(|| "rollout has no session id metadata".to_string())?;
        let cwd = self
            .cwd
            .ok_or_else(|| "rollout has no cwd metadata".to_string())?;
        let working_directory = cwd.to_string_lossy().into_owned();
        let summary = self.summary.or(self.fallback_summary).unwrap_or_default();
        let started_at = self
            .first_timestamp
            .or(self.metadata_timestamp)
            .unwrap_or(self.fallback_time);
        let ended_at = self
            .last_timestamp
            .unwrap_or(self.fallback_time)
            .max(started_at);
        Ok(ParsedRollout {
            thread: CodexThreadSummary {
                thread_id,
                working_directory,
                title: summary.clone(),
                summary,
                started_at,
                ended_at,
                branch: self.branch,
                recorded_sha: self.recorded_sha,
                is_active: false,
                user_round_count: self.user_round_count,
                agent_message_count: self.agent_message_count,
                tool_call_count: self.tool_call_count,
                total_tokens: self.total_tokens,
                metrics_complete: self.metrics_complete
                    && self.first_timestamp.is_some()
                    && self.last_timestamp.is_some(),
                commit_occurrence_count: 0,
            },
            cwd,
            has_record_endpoints: self.first_timestamp.is_some() && self.last_timestamp.is_some(),
            is_internal_subagent: self.is_internal_subagent,
        })
    }
}

fn rollout_is_user_visible(rollout: &ParsedRollout) -> bool {
    !rollout.is_internal_subagent
}

fn event_message_text(payload: &Value) -> Option<String> {
    payload
        .get("message")
        .and_then(extract_text)
        .or_else(|| payload.get("text").and_then(extract_text))
}

fn genuine_user_event(payload: &Value) -> Option<String> {
    (payload.get("type").and_then(Value::as_str) == Some("user_message"))
        .then(|| event_message_text(payload))
        .flatten()
        .map(|text| normalize_summary(&text))
        .filter(|text| is_meaningful_user_text(text))
}

fn visible_chat_message(value: &Value) -> Option<(ChatTurnRole, String, Option<u64>)> {
    let payload = value.get("payload").unwrap_or(value);
    let payload_type = payload.get("type").and_then(Value::as_str);
    let (role, text) = match payload_type {
        Some("user_message") => (ChatTurnRole::User, genuine_user_event(payload)?),
        Some("agent_message") => (
            ChatTurnRole::Assistant,
            event_message_text(payload).map(|text| normalize_summary(&text))?,
        ),
        Some("message") if payload.get("role").and_then(Value::as_str) == Some("assistant") => (
            ChatTurnRole::Assistant,
            payload
                .get("content")
                .and_then(extract_text)
                .map(|text| normalize_summary(&text))?,
        ),
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    let timestamp = value_alias(value, &["timestamp"])
        .or_else(|| value_alias(payload, &["timestamp"]))
        .and_then(parse_timestamp_value);
    Some((role, truncate_chars(&text, 240), timestamp))
}

fn is_meaningful_user_text(value: &str) -> bool {
    let trimmed = value.trim_start();
    !trimmed.is_empty()
        && ![
            "<recommended_plugins>",
            "<skill>",
            "<environment_context>",
            "<permissions instructions>",
            "<app-context>",
        ]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

#[cfg(test)]
fn parse_thread_detail_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    cursor: Option<usize>,
    limit: usize,
) -> CodexThreadDetailsPage {
    let limit = limit.clamp(1, 50);
    let mut visible = Vec::new();
    let mut previous: Option<(ChatTurnRole, String, Option<u64>)> = None;
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some((role, preview, timestamp)) = visible_chat_message(&value) else {
            continue;
        };
        if previous.as_ref() == Some(&(role, preview.clone(), timestamp)) {
            continue;
        }
        previous = Some((role, preview.clone(), timestamp));
        let ordinal = visible.len();
        visible.push(ChatTurnPreview {
            ordinal,
            role,
            timestamp,
            preview,
        });
    }
    let (start, end, next_cursor) = latest_turn_bounds(visible.len(), cursor, limit);
    let turns = visible[start..end].to_vec();
    CodexThreadDetailsPage {
        thread_id: String::new(),
        turns,
        next_cursor,
    }
}

fn latest_turn_bounds(
    total: usize,
    cursor: Option<usize>,
    limit: usize,
) -> (usize, usize, Option<usize>) {
    let newer_turns = cursor.unwrap_or(0).min(total);
    let end = total.saturating_sub(newer_turns);
    let start = end.saturating_sub(limit);
    let next_cursor = (start > 0).then_some(newer_turns.saturating_add(end - start));
    (start, end, next_cursor)
}

fn scan_thread_detail_file(
    path: &Path,
    mut visit: impl FnMut(ChatTurnPreview) -> bool,
) -> Result<usize, String> {
    let file = File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut ordinal = 0usize;
    let mut bytes = Vec::new();
    let mut previous: Option<(ChatTurnRole, String, Option<u64>)> = None;
    loop {
        let record_start = reader
            .stream_position()
            .map_err(|error| format!("read '{}': {error}", path.display()))?;
        let value = match read_bounded_line(&mut reader, &mut bytes, MAX_LINE_BYTES)
            .map_err(|error| format!("read '{}': {error}", path.display()))?
        {
            BoundedLine::Eof => break,
            BoundedLine::Oversized => {
                let record_end = reader
                    .stream_position()
                    .map_err(|error| format!("read '{}': {error}", path.display()))?;
                recover_oversized_record(&mut reader, record_start, record_end).ok()
            }
            BoundedLine::Line => serde_json::from_slice::<Value>(&bytes).ok(),
        };
        let Some(value) = value else {
            continue;
        };
        let Some((role, preview, timestamp)) = visible_chat_message(&value) else {
            continue;
        };
        if previous.as_ref() == Some(&(role, preview.clone(), timestamp)) {
            continue;
        }
        previous = Some((role, preview.clone(), timestamp));
        let turn = ChatTurnPreview {
            ordinal,
            role,
            timestamp,
            preview,
        };
        ordinal = ordinal.saturating_add(1);
        if !visit(turn) {
            break;
        }
    }
    Ok(ordinal)
}

fn parse_thread_detail_file(
    path: &Path,
    cursor: Option<usize>,
    limit: usize,
) -> Result<CodexThreadDetailsPage, String> {
    let limit = limit.clamp(1, 50);
    let retained_limit = cursor.unwrap_or(0).saturating_add(limit);
    let mut recent = VecDeque::new();
    let total = scan_thread_detail_file(path, |turn| {
        recent.push_back(turn);
        if recent.len() > retained_limit {
            recent.pop_front();
        }
        true
    })?;
    let (start, end, next_cursor) = latest_turn_bounds(total, cursor, limit);
    let turns = recent
        .into_iter()
        .filter(|turn| turn.ordinal >= start && turn.ordinal < end)
        .collect();
    Ok(CodexThreadDetailsPage {
        thread_id: String::new(),
        next_cursor,
        turns,
    })
}

fn extract_user_message(value: &Value) -> Option<String> {
    let payload = value.get("payload").unwrap_or(value);
    if payload.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    if let Some(text) = payload.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(content) = payload.get("content") {
        return extract_text(content);
    }
    payload.get("message").and_then(extract_text)
}

fn extract_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items.iter().filter_map(extract_text).collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join(" "))
        }
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| object.get("content").and_then(extract_text)),
        _ => None,
    }
}

// Oversized rollouts commonly contain base64 images, compaction snapshots, or
// tool output. This projection keeps only fields used by history/details;
// serde consumes unknown values through IgnoredAny without retaining them.
#[derive(Default, Serialize)]
#[serde(transparent)]
struct ProjectedText(String);

impl<'de> Deserialize<'de> for ProjectedText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ProjectedTextVisitor;

        impl<'de> Visitor<'de> for ProjectedTextVisitor {
            type Value = ProjectedText;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("text or a nested message-content value")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ProjectedText(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(ProjectedText(value))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(ProjectedText::default())
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(ProjectedText::default())
            }

            fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
                Ok(ProjectedText::default())
            }

            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
                Ok(ProjectedText::default())
            }

            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
                Ok(ProjectedText::default())
            }

            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
                Ok(ProjectedText::default())
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut parts = Vec::new();
                while let Some(ProjectedText(text)) = sequence.next_element::<ProjectedText>()? {
                    if !text.is_empty() {
                        parts.push(text);
                    }
                }
                Ok(ProjectedText(parts.join(" ")))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut parts = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    if matches!(key.as_str(), "text" | "content" | "message") {
                        let ProjectedText(text) = map.next_value::<ProjectedText>()?;
                        if !text.is_empty() {
                            parts.push(text);
                        }
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(ProjectedText(parts.join(" ")))
            }
        }

        deserializer.deserialize_any(ProjectedTextVisitor)
    }
}

#[derive(Deserialize, Serialize)]
struct ProjectedGit {
    #[serde(default, alias = "git_branch", skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(
        default,
        alias = "commit",
        alias = "sha",
        alias = "git_commit",
        skip_serializing_if = "Option::is_none"
    )]
    commit_hash: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct ProjectedTokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    total_tokens: Option<u64>,
}

#[derive(Deserialize, Serialize)]
struct ProjectedInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    total_token_usage: Option<ProjectedTokenUsage>,
}

#[derive(Deserialize, Serialize)]
struct ProjectedRecord {
    #[serde(
        default,
        alias = "created_at",
        alias = "started_at",
        skip_serializing_if = "Option::is_none"
    )]
    timestamp: Option<Value>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    record_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(
        default,
        alias = "thread_id",
        alias = "session_id",
        skip_serializing_if = "Option::is_none"
    )]
    id: Option<String>,
    #[serde(
        default,
        alias = "working_directory",
        alias = "project_path",
        skip_serializing_if = "Option::is_none"
    )]
    cwd: Option<String>,
    #[serde(default, alias = "git_branch", skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(
        default,
        alias = "git_commit",
        alias = "commit_hash",
        skip_serializing_if = "Option::is_none"
    )]
    commit_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<ProjectedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<ProjectedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<ProjectedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git: Option<ProjectedGit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    info: Option<ProjectedInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload: Option<Box<ProjectedRecord>>,
}

fn recover_oversized_record(
    reader: &mut BufReader<File>,
    record_start: u64,
    record_end: u64,
) -> Result<Value, String> {
    reader
        .seek(SeekFrom::Start(record_start))
        .map_err(|error| error.to_string())?;
    let parsed = {
        let limited = Read::by_ref(reader).take(record_end.saturating_sub(record_start));
        serde_json::from_reader::<_, ProjectedRecord>(limited).map_err(|error| error.to_string())
    };
    reader
        .seek(SeekFrom::Start(record_end))
        .map_err(|error| error.to_string())?;
    serde_json::to_value(parsed?).map_err(|error| error.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundedLine {
    Eof,
    Line,
    Oversized,
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    output: &mut Vec<u8>,
    max_bytes: usize,
) -> io::Result<BoundedLine> {
    output.clear();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if output.is_empty() && !oversized {
                return Ok(BoundedLine::Eof);
            }
            break;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(available.len());
        if !oversized {
            if output.len().saturating_add(content_len) > max_bytes {
                oversized = true;
                output.clear();
            } else {
                output.extend_from_slice(&available[..content_len]);
            }
        }
        let consumed = content_len + usize::from(newline.is_some());
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    if oversized {
        return Ok(BoundedLine::Oversized);
    }
    if output.last() == Some(&b'\r') {
        output.pop();
    }
    Ok(BoundedLine::Line)
}

fn read_bounded_lines(
    path: &Path,
    max_lines: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|error| format!("open '{}': {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut bytes = Vec::new();
    let mut processed = 0usize;
    while processed < max_lines {
        let state = read_bounded_line(&mut reader, &mut bytes, MAX_LINE_BYTES)
            .map_err(|error| format!("read '{}': {error}", path.display()))?;
        if state == BoundedLine::Eof {
            break;
        }
        processed += 1;
        if state == BoundedLine::Oversized {
            warnings.push(format!(
                "Ignored oversized JSONL line in '{}'",
                path.display()
            ));
            continue;
        }
        if bytes.is_empty() {
            continue;
        }
        match String::from_utf8(bytes.clone()) {
            Ok(line) if serde_json::from_str::<Value>(&line).is_ok() => lines.push(line),
            Ok(_) => warnings.push(format!(
                "Ignored malformed JSONL line in '{}'",
                path.display()
            )),
            Err(_) => warnings.push(format!(
                "Ignored non-UTF-8 JSONL line in '{}'",
                path.display()
            )),
        }
    }
    let has_more = reader.fill_buf().is_ok_and(|buffer| !buffer.is_empty());
    if max_lines != usize::MAX && processed == max_lines && has_more {
        warnings.push(format!(
            "Stopped reading '{}' after {max_lines} JSONL lines",
            path.display()
        ));
    }
    Ok(lines)
}

#[cfg(test)]
fn discover_git_history(
    project_root: &Path,
    requested_branch: Option<&str>,
    before_commit: Option<&str>,
    limit: usize,
) -> Result<Option<GitDiscovery>, String> {
    discover_git_history_with_recorded_branches(
        project_root,
        requested_branch,
        before_commit,
        limit,
        &BTreeSet::new(),
    )
}

fn discover_git_history_with_recorded_branches(
    project_root: &Path,
    requested_branch: Option<&str>,
    before_commit: Option<&str>,
    limit: usize,
    recorded_branches: &BTreeSet<String>,
) -> Result<Option<GitDiscovery>, String> {
    let probe = git_output(project_root, &["rev-parse", "--is-inside-work-tree"])?;
    if !probe.status.success() || String::from_utf8_lossy(&probe.stdout).trim() != "true" {
        return Ok(None);
    }
    let current = git_output(
        project_root,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )
    .ok()
    .filter(|output| output.status.success())
    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    .filter(|value| !value.is_empty());
    let output = git_output(
        project_root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )?;
    if !output.status.success() {
        return Err(git_error("list branches", &output));
    }
    let available_names = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    let mut names = available_names.clone();
    if let Some(current) = &current {
        names.insert(current.clone());
    }
    names.extend(recorded_branches.iter().cloned());
    let selected = if let Some(requested) = requested_branch {
        if !names.contains(requested) {
            return Err(
                "branch is not one of the repository's enumerated local branches".to_string(),
            );
        }
        requested.to_string()
    } else if let Some(current) = &current {
        current.clone()
    } else {
        names
            .iter()
            .next()
            .cloned()
            .unwrap_or_else(|| "HEAD".to_string())
    };
    let branches = names
        .into_iter()
        .map(|name| GitBranchSummary {
            is_current: current.as_deref() == Some(name.as_str()),
            available: available_names.contains(&name) || current.as_deref() == Some(name.as_str()),
            name,
        })
        .collect::<Vec<_>>();

    // An unborn current branch has a symbolic HEAD but no refs/heads entry
    // until its first commit. Keep it visible as the current local branch,
    // while avoiding a git log call against a revision that does not exist.
    let selected_ref_available = available_names.contains(&selected);
    let all = if selected_ref_available {
        load_first_parent_commits(project_root, &selected, MAX_GIT_COMMITS + 1)?
    } else {
        Vec::new()
    };
    let start = if let Some(cursor) = before_commit {
        validate_sha(cursor)?;
        all.iter()
            .position(|commit| commit.sha.eq_ignore_ascii_case(cursor))
            .map(|position| position + 1)
            .ok_or_else(|| {
                "commit cursor is not on the selected branch's first-parent history".to_string()
            })?
    } else {
        0
    };
    let limit = limit.clamp(1, MAX_COMMIT_LIMIT);
    let commits = all
        .iter()
        .skip(start)
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    let next_cursor = if start + commits.len() < all.len() {
        commits.last().map(|commit| commit.sha.clone())
    } else {
        None
    };
    Ok(Some(GitDiscovery {
        selected_branch: selected,
        selected_ref_available,
        branches,
        commits,
        next_cursor,
    }))
}

fn load_first_parent_commits(
    project_root: &Path,
    branch: &str,
    limit: usize,
) -> Result<Vec<GitCommitSummary>, String> {
    let count = limit.min(MAX_GIT_COMMITS + 1).to_string();
    let branch_ref = if branch == "HEAD" {
        "HEAD".to_string()
    } else {
        format!("refs/heads/{branch}")
    };
    let output = git_output(
        project_root,
        &[
            "log",
            "--first-parent",
            "--date-order",
            "--format=%H%x1f%ct%x1f%s",
            "-n",
            &count,
            &branch_ref,
            "--",
        ],
    )?;
    if !output.status.success() {
        return Err(git_error("read first-parent history", &output));
    }
    let mut commits = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.splitn(3, '\u{1f}');
        let Some(sha) = fields.next() else { continue };
        let Some(time) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        let subject = fields.next().unwrap_or_default().to_string();
        if validate_sha(sha).is_err() {
            continue;
        }
        commits.push(GitCommitSummary {
            sha: sha.to_ascii_lowercase(),
            short_sha: sha.chars().take(12).collect(),
            committed_at: time,
            subject,
            thread_refs: Vec::new(),
        });
    }
    Ok(commits)
}

fn map_threads_to_commits(
    threads: &[CodexThreadSummary],
    commits: &mut [GitCommitSummary],
    selected_branch: &str,
) -> MappingResult {
    let mut mapped_threads = BTreeSet::new();
    let mut unmapped = Vec::new();
    // The rail arrives newest-first, so a commit's first-parent descendants
    // sit at lower indices than the commit itself.
    let rail_index = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.sha.clone(), index))
        .collect::<BTreeMap<_, _>>();
    for thread in threads {
        // A recorded branch is authoritative routing metadata. Rollouts that
        // predate branch capture stay eligible as a best-effort fallback,
        // but a thread from another named branch must never appear on this
        // branch's commit rail.
        if thread
            .branch
            .as_deref()
            .is_some_and(|branch| branch != selected_branch)
        {
            continue;
        }
        let mut matched = Vec::new();
        for (index, commit) in commits.iter().enumerate() {
            if commit.committed_at >= thread.started_at && commit.committed_at <= thread.ended_at {
                matched.push(index);
            }
        }
        if matched.is_empty() {
            // A session no commit overlapped attaches to the commit that
            // captured its work afterwards, however long that took. When the
            // session's recorded HEAD is on this rail only its first-parent
            // descendants qualify, walking from the recorded commit toward
            // the tip; a commit that predates the recorded checkout must not
            // capture the session even if its timestamp is later. Without a
            // usable recorded HEAD, the earliest later commit stands in.
            let successor = match thread
                .recorded_sha
                .as_deref()
                .map(str::to_ascii_lowercase)
                .and_then(|sha| rail_index.get(&sha).copied())
            {
                Some(recorded_index) => (0..recorded_index)
                    .rev()
                    .find(|&index| commits[index].committed_at > thread.ended_at),
                None => commits
                    .iter()
                    .enumerate()
                    .filter(|(_, commit)| commit.committed_at > thread.ended_at)
                    .min_by_key(|(_, commit)| commit.committed_at)
                    .map(|(index, _)| index),
            };
            matched.extend(successor);
        }
        if matched.is_empty() {
            let reason = if commits.is_empty() {
                "The selected branch has no available first-parent commits"
            } else {
                "No commit has been made on this branch since the session ended"
            };
            unmapped.push(UnmappedThreadReference {
                thread_id: thread.thread_id.clone(),
                reason: reason.to_string(),
            });
            continue;
        }
        mapped_threads.insert(thread.thread_id.clone());
        for index in matched {
            commits[index].thread_refs.push(CommitThreadReference {
                thread_id: thread.thread_id.clone(),
            });
        }
    }
    let thread_order = threads
        .iter()
        .map(|thread| (thread.thread_id.as_str(), thread))
        .collect::<BTreeMap<_, _>>();
    for commit in commits.iter_mut() {
        commit.thread_refs.sort_by(|left, right| {
            match (
                thread_order.get(left.thread_id.as_str()),
                thread_order.get(right.thread_id.as_str()),
            ) {
                (Some(left_thread), Some(right_thread)) => right_thread
                    .ended_at
                    .cmp(&left_thread.ended_at)
                    .then_with(|| left_thread.started_at.cmp(&right_thread.started_at))
                    .then_with(|| left.thread_id.cmp(&right.thread_id)),
                _ => left.thread_id.cmp(&right.thread_id),
            }
        });
    }
    let reference_count = commits.iter().map(|commit| commit.thread_refs.len()).sum();
    MappingResult {
        unmapped,
        unique_thread_count: mapped_threads.len(),
        reference_count,
    }
}

fn discover_github_repository_url(project_root: &Path) -> Option<String> {
    let output = git_output(
        project_root,
        &["config", "--get-regexp", r"^remote\..*\.url$"],
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }

    let mut fallback = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some(separator) = line.find(char::is_whitespace) else {
            continue;
        };
        let key = &line[..separator];
        let value = line[separator..].trim();
        let Some(remote_name) = key
            .strip_prefix("remote.")
            .and_then(|value| value.strip_suffix(".url"))
        else {
            continue;
        };
        let Some(repository_url) = normalize_github_repository_url(value) else {
            continue;
        };
        if remote_name.eq_ignore_ascii_case("origin") {
            return Some(repository_url);
        }
        fallback.get_or_insert(repository_url);
    }
    fallback
}

pub(super) fn normalize_github_repository_url(remote_url: &str) -> Option<String> {
    let remote_url = remote_url.trim();
    if remote_url.is_empty()
        || remote_url
            .chars()
            .any(|character| matches!(character, '\r' | '\n' | '\0' | '?' | '#' | '\\'))
    {
        return None;
    }

    let path = if let Some((scheme, remainder)) = remote_url.split_once("://") {
        if !matches!(
            scheme.to_ascii_lowercase().as_str(),
            "https" | "http" | "ssh" | "git"
        ) {
            return None;
        }
        let (authority, path) = remainder.split_once('/')?;
        let host_with_port = authority.rsplit('@').next()?;
        let host = if let Some((host, port)) = host_with_port.rsplit_once(':') {
            if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            host
        } else {
            host_with_port
        };
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        path
    } else {
        let (authority, path) = remote_url.split_once(':')?;
        let (_, host) = authority.rsplit_once('@')?;
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        path
    };

    let path = path.trim_matches('/');
    let mut segments = path.split('/');
    let owner = segments.next()?;
    let repository = segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    let repository = repository.strip_suffix(".git").unwrap_or(repository);
    if !valid_github_path_segment(owner) || !valid_github_path_segment(repository) {
        return None;
    }
    Some(format!("https://github.com/{owner}/{repository}"))
}

fn valid_github_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn git_output(root: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("run Git for '{}': {error}", root.display()))
}

fn git_error(action: &str, output: &Output) -> String {
    let detail = String::from_utf8_lossy(&output.stderr);
    format!("Git could not {action}: {}", detail.trim())
}

fn launch_terminal_resume(
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
    before_launch: impl FnOnce(&str),
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let codex_cli = resolve_codex_cli()?;
        let shell = terminal_resume_command(&codex_cli, thread_id, project_root, codex_home)?;
        before_launch(&shell);
        let script = format!(
            "tell application \"Terminal\" to do script \"{}\"",
            apple_script_string(&shell)
        );
        let output = Command::new("/usr/bin/osascript")
            .args([
                "-e",
                &script,
                "-e",
                "tell application \"Terminal\" to activate",
            ])
            .output()
            .map_err(|error| format!("open Terminal: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Terminal could not resume the Codex thread: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (thread_id, project_root, codex_home, before_launch);
        Err("Open in Terminal is currently supported only on macOS".to_string())
    }
}

fn launch_codex_app(
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
    mut before_launch: impl FnMut(&str),
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let deep_links = codex_app_deep_links(thread_id, project_root, codex_home)?;
        let commands = deep_links
            .each_ref()
            .map(|deep_link| codex_app_open_command(codex_home, deep_link));

        // Existing-thread deep links ignore a workspace path. Send a dedicated
        // new-thread route first, then wait for that profile to persist the
        // project selection before replacing the composer with the target thread.
        let state_path = codex_home.join(".codex-global-state.json");
        let initial_state = file_stamp(&state_path);
        before_launch(&commands[0]);
        run_codex_app_deep_link(codex_home, &deep_links[0], "project")?;
        wait_for_file_change(&state_path, initial_state);

        before_launch(&commands[1]);
        run_codex_app_deep_link(codex_home, &deep_links[1], "thread")?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (thread_id, project_root, codex_home, before_launch);
        Err("Open in Codex is currently supported only on macOS".to_string())
    }
}

#[cfg(target_os = "macos")]
fn run_codex_app_deep_link(codex_home: &Path, deep_link: &str, target: &str) -> Result<(), String> {
    let environment = format!("CODEX_HOME={}", codex_home.to_string_lossy());
    let output = Command::new("/usr/bin/open")
        .args([
            "-n",
            "-a",
            "/Applications/ChatGPT.app",
            "--env",
            &environment,
            deep_link,
        ])
        .output()
        .map_err(|error| format!("open the Codex desktop app {target}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Codex desktop app could not open the {target}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn file_stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let metadata = fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

#[cfg(target_os = "macos")]
fn wait_for_file_change(path: &Path, initial: Option<(u64, SystemTime)>) {
    let started = std::time::Instant::now();
    while started.elapsed() < CODEX_PROJECT_ROUTE_TIMEOUT {
        if file_stamp(path) != initial {
            return;
        }
        std::thread::sleep(CODEX_PROJECT_ROUTE_POLL_INTERVAL);
    }
}

#[cfg(target_os = "macos")]
fn resolve_codex_cli() -> Result<PathBuf, String> {
    // GUI applications inherit launchd's sparse PATH. A login shell sees the
    // same Homebrew/npm/user configuration as the Terminal window we open.
    let output = Command::new("/bin/zsh")
        .args(["-lc", "command -v codex"])
        .output()
        .map_err(|error| format!("check for Codex CLI in the login shell: {error}"))?;
    if !output.status.success() {
        return Err(
            "Codex CLI is unavailable in the login shell; install it or use Open in Codex"
                .to_string(),
        );
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let path = PathBuf::from(value.lines().next().unwrap_or_default().trim());
    if !path.is_absolute() {
        return Err("Codex CLI did not resolve to an absolute executable path".to_string());
    }
    let resolved = fs::canonicalize(&path)
        .map_err(|error| format!("resolve Codex CLI '{}': {error}", path.display()))?;
    if !resolved.is_file() {
        return Err(format!(
            "Codex CLI '{}' is not an executable file",
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn terminal_resume_command(
    codex_cli: &Path,
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
) -> Result<String, String> {
    validate_thread_uuid(thread_id)?;
    if !codex_cli.is_absolute() || !project_root.is_absolute() || !codex_home.is_absolute() {
        return Err("Codex CLI, project root, and Codex home must be absolute".to_string());
    }
    Ok(format!(
        "CODEX_HOME={} {} resume {} -C {}",
        shell_quote(&codex_home.to_string_lossy()),
        shell_quote(&codex_cli.to_string_lossy()),
        shell_quote(thread_id),
        shell_quote(&project_root.to_string_lossy())
    ))
}

fn codex_app_deep_links(
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
) -> Result<[String; 2], String> {
    validate_thread_uuid(thread_id)?;
    if !project_root.is_absolute() || !codex_home.is_absolute() {
        return Err("Project root and Codex home must be absolute".to_string());
    }
    Ok([
        format!(
            "codex://threads/new?path={}",
            percent_encode_uri_component(&project_root.to_string_lossy())
        ),
        format!("codex://threads/{thread_id}"),
    ])
}

/// The invocation `open_in_codex` performs for a project, rendered as a shell
/// line.  Logs quote this so the command they teach is the one that runs.
pub(crate) fn codex_app_project_open_command(
    codex_home: &Path,
    project_root: &Path,
) -> Option<String> {
    if !codex_home.is_absolute() || !project_root.is_absolute() {
        return None;
    }
    let deep_link = format!(
        "codex://threads/new?path={}",
        percent_encode_uri_component(&project_root.to_string_lossy())
    );
    Some(codex_app_open_command(codex_home, &deep_link))
}

fn codex_app_open_command(codex_home: &Path, deep_link: &str) -> String {
    format!(
        "'/usr/bin/open' -n -a '/Applications/ChatGPT.app' --env {} {}",
        shell_quote(&format!("CODEX_HOME={}", codex_home.to_string_lossy())),
        shell_quote(deep_link)
    )
}

#[cfg(test)]
fn codex_app_commands(
    thread_id: &str,
    project_root: &Path,
    codex_home: &Path,
) -> Result<[String; 2], String> {
    Ok(codex_app_deep_links(thread_id, project_root, codex_home)?
        .each_ref()
        .map(|deep_link| codex_app_open_command(codex_home, deep_link)))
}

fn percent_encode_uri_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

fn validate_thread_uuid(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
    valid
        .then_some(())
        .ok_or_else(|| "Codex thread id must be a UUID".to_string())
}

fn validate_sha(value: &str) -> Result<(), String> {
    ((value.len() == 40 || value.len() == 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(())
        .ok_or_else(|| "commit cursor must be a full hexadecimal Git object id".to_string())
}

fn cwd_belongs_to_project(cwd: &Path, project_root: &Path) -> bool {
    if !cwd.is_absolute()
        || cwd
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return false;
    }
    let resolved = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    resolved == project_root || resolved.starts_with(project_root)
}

fn value_alias<'a>(value: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| value.get(*name))
}

fn string_alias<'a>(value: &'a Value, names: &[&str]) -> Option<&'a str> {
    value_alias(value, names).and_then(Value::as_str)
}

fn parse_timestamp_value(value: &Value) -> Option<u64> {
    if let Some(number) = value.as_u64() {
        return Some(if number > 10_000_000_000 {
            number / 1000
        } else {
            number
        });
    }
    if let Some(number) = value.as_i64() {
        return (number >= 0).then_some(number as u64).map(|value| {
            if value > 10_000_000_000 {
                value / 1000
            } else {
                value
            }
        });
    }
    value.as_str().and_then(parse_rfc3339)
}

fn parse_rfc3339(value: &str) -> Option<u64> {
    let (date, rest) = value.split_once('T')?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i64>().ok()?;
    let month = date_parts.next()?.parse::<u32>().ok()?;
    let day = date_parts.next()?.parse::<u32>().ok()?;
    let zone_index = rest
        .char_indices()
        .skip(1)
        .find(|(_, character)| matches!(character, 'Z' | '+' | '-'))
        .map(|(index, _)| index)?;
    let (clock, zone) = rest.split_at(zone_index);
    let mut clock_parts = clock.split(':');
    let hour = clock_parts.next()?.parse::<u32>().ok()?;
    let minute = clock_parts.next()?.parse::<u32>().ok()?;
    let second = clock_parts.next()?.split('.').next()?.parse::<u32>().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let offset = if zone == "Z" {
        0i64
    } else {
        let sign = if zone.starts_with('+') {
            1
        } else if zone.starts_with('-') {
            -1
        } else {
            return None;
        };
        let mut parts = zone[1..].split(':');
        let hours = parts.next()?.parse::<i64>().ok()?;
        let minutes = parts.next()?.parse::<i64>().ok()?;
        if hours > 23 || minutes > 59 {
            return None;
        }
        sign * (hours * 3600 + minutes * 60)
    };
    let days = days_from_civil(year, month, day);
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))?
        .checked_sub(offset)?;
    (seconds >= 0).then_some(seconds as u64)
}

// Howard Hinnant's proleptic Gregorian conversion, offset to Unix epoch.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let month_prime = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn normalize_summary(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut result = value
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    result.push('…');
    result
}

fn short_id(value: &str) -> &str {
    value.get(..8).unwrap_or(value)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "macos")]
fn apple_script_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn modified_secs(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn modified_nanos(metadata: &fs::Metadata) -> Option<u64> {
    let duration = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    u64::try_from(duration.as_nanos()).ok()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn session_mtime_is_active(now: u64, modified: u64) -> bool {
    modified <= now.saturating_add(ACTIVE_WINDOW_SECS)
        && now.saturating_sub(modified) <= ACTIVE_WINDOW_SECS
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::io::{BufWriter, Write};
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn first_user_summary_ignores_metadata_and_normalizes_visible_text() {
        let bytes = [
            json!({"type":"session_meta","payload":{"id":"thread-a","cwd":"/tmp/project"}}),
            json!({"type":"response_item","payload":{"role":"user","content":[{"type":"input_text","text":"  Analyze   the graph  "} ]}}),
            json!({"type":"response_item","payload":{"role":"assistant","content":[{"type":"output_text","text":"Done"}]}}),
        ]
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        assert_eq!(
            first_codex_user_summary(bytes.as_bytes()).as_deref(),
            Some("Analyze the graph")
        );
    }

    fn commit(sha: &str, time: u64) -> GitCommitSummary {
        GitCommitSummary {
            sha: sha.repeat(40 / sha.len()),
            short_sha: sha.repeat(12 / sha.len()),
            committed_at: time,
            subject: format!("commit {time}"),
            thread_refs: Vec::new(),
        }
    }

    fn thread(id: &str, start: u64, end: u64) -> CodexThreadSummary {
        CodexThreadSummary {
            thread_id: id.to_string(),
            working_directory: "/tmp/project".to_string(),
            title: id.to_string(),
            summary: String::new(),
            started_at: start,
            ended_at: end,
            branch: Some("main".to_string()),
            recorded_sha: None,
            is_active: false,
            user_round_count: 0,
            agent_message_count: 0,
            tool_call_count: 0,
            total_tokens: None,
            metrics_complete: true,
            commit_occurrence_count: 0,
        }
    }

    #[test]
    fn mapping_attaches_all_inclusive_commits_and_counts_unique_threads() {
        let mut commits = vec![commit("a", 100), commit("b", 150), commit("c", 200)];
        let threads = vec![thread("one", 100, 200), thread("two", 150, 150)];
        let result = map_threads_to_commits(&threads, &mut commits, "main");

        assert_eq!(result.unmapped, Vec::<UnmappedThreadReference>::new());
        assert_eq!(result.unique_thread_count, 2);
        assert_eq!(result.reference_count, 4);
        assert_eq!(commits[1].thread_refs.len(), 2);
    }

    #[test]
    fn mapping_attaches_the_earliest_later_commit_without_a_time_cap() {
        let mut commits = vec![commit("b", 300_000), commit("a", 200_099)];
        let far = thread("far", 0, 99);
        let result = map_threads_to_commits(&[far], &mut commits, "main");

        assert!(result.unmapped.is_empty());
        assert!(commits[0].thread_refs.is_empty());
        assert_eq!(commits[1].thread_refs.len(), 1);
        assert_eq!(commits[1].thread_refs[0].thread_id, "far");
    }

    #[test]
    fn mapping_walks_first_parent_descendants_of_the_recorded_head() {
        let mut commits = vec![commit("c", 300), commit("b", 200), commit("a", 50)];
        let mut recorded = thread("recorded", 60, 100);
        recorded.recorded_sha = Some(commit("a", 50).sha.to_ascii_uppercase());
        let result = map_threads_to_commits(&[recorded], &mut commits, "main");

        assert!(result.unmapped.is_empty());
        assert!(commits[0].thread_refs.is_empty());
        assert_eq!(commits[1].thread_refs.len(), 1);
        assert_eq!(commits[1].thread_refs[0].thread_id, "recorded");
        assert!(commits[2].thread_refs.is_empty());
    }

    #[test]
    fn mapping_leaves_a_session_at_the_recorded_tip_uncommitted_until_the_next_commit() {
        let mut commits = vec![commit("a", 50)];
        let mut at_tip = thread("at-tip", 60, 100);
        at_tip.recorded_sha = Some(commit("a", 50).sha);
        let result = map_threads_to_commits(&[at_tip], &mut commits, "main");

        assert!(commits[0].thread_refs.is_empty());
        assert_eq!(result.unmapped.len(), 1);
        assert_eq!(
            result.unmapped[0].reason,
            "No commit has been made on this branch since the session ended"
        );
    }

    #[test]
    fn mapping_never_attaches_a_commit_below_the_recorded_head() {
        // "b" sits below the recorded tip on the rail; its later timestamp
        // (clock skew) must not capture the session.
        let mut commits = vec![commit("a", 50), commit("b", 150)];
        let mut skewed = thread("skewed", 60, 100);
        skewed.recorded_sha = Some(commit("a", 50).sha);
        let result = map_threads_to_commits(&[skewed], &mut commits, "main");

        assert!(commits[0].thread_refs.is_empty());
        assert!(commits[1].thread_refs.is_empty());
        assert_eq!(result.unmapped.len(), 1);
    }

    #[test]
    fn mapping_falls_back_to_time_when_the_recorded_head_is_not_on_the_rail() {
        let mut commits = vec![commit("b", 200_000)];
        let mut rewritten = thread("rewritten", 1, 2);
        rewritten.recorded_sha = Some("f".repeat(40));
        let result = map_threads_to_commits(&[rewritten], &mut commits, "main");

        assert!(result.unmapped.is_empty());
        assert_eq!(commits[0].thread_refs.len(), 1);
        assert_eq!(commits[0].thread_refs[0].thread_id, "rewritten");
    }

    #[test]
    fn mapping_attaches_a_fork_to_the_next_commit_after_its_recorded_base() {
        let mut commits = vec![commit("b", 500_000), commit("a", 100)];
        let mut fork = thread("fork", 200, 200);
        fork.recorded_sha = Some(commit("a", 100).sha);
        let result = map_threads_to_commits(&[fork], &mut commits, "main");

        assert!(result.unmapped.is_empty());
        assert_eq!(commits[0].thread_refs.len(), 1);
        assert_eq!(commits[0].thread_refs[0].thread_id, "fork");
        assert!(commits[1].thread_refs.is_empty());
    }

    #[test]
    fn mapping_never_attaches_a_named_thread_to_another_branch() {
        let mut other = thread("feature-thread", 1, 300);
        other.branch = Some("feature/a".to_string());
        let mut unknown = thread("unknown-thread", 1, 300);
        unknown.branch = None;
        let mut commits = vec![commit("a", 200)];

        let result = map_threads_to_commits(&[other, unknown], &mut commits, "main");

        assert_eq!(commits[0].thread_refs.len(), 1);
        assert_eq!(commits[0].thread_refs[0].thread_id, "unknown-thread");
        assert_eq!(result.unique_thread_count, 1);
        assert!(result.unmapped.is_empty());
    }

    #[test]
    fn rollout_parsing_accepts_metadata_aliases_and_first_user_message() {
        let lines = vec![
            json!({"timestamp":"2026-07-18T16:20:00Z","type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project","git":{"branch":"feature/x","commit_hash":"abc"}}}).to_string(),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"  Build   the history page please  "}]}}).to_string(),
        ];
        let parsed = parse_rollout_lines(lines.iter().map(String::as_str), 123).unwrap();

        assert_eq!(
            parsed.thread.thread_id,
            "019f742a-a206-7932-876c-9db8d8ce575a"
        );
        assert_eq!(parsed.cwd, Path::new("/tmp/project"));
        assert_eq!(parsed.thread.working_directory, "/tmp/project");
        assert_eq!(parsed.thread.branch.as_deref(), Some("feature/x"));
        assert_eq!(parsed.thread.recorded_sha.as_deref(), Some("abc"));
        assert_eq!(parsed.thread.summary, "Build the history page please");
        assert_eq!(parsed.thread.started_at, 1_784_391_600);
    }

    #[test]
    fn guardian_subagent_rollouts_are_not_user_visible_chats() {
        let lines = vec![
            json!({
                "timestamp": 100,
                "type": "session_meta",
                "payload": {
                    "id": "guardian-thread",
                    "cwd": "/tmp/project",
                    "thread_source": "subagent",
                    "source": {"subagent": {"other": "guardian"}},
                    "parent_thread_id": "user-thread"
                }
            })
            .to_string(),
            json!({
                "timestamp": 101,
                "type": "event_msg",
                "payload": {
                    "type": "user_message",
                    "message": "The following is the Codex agent history whose request action you are assessing."
                }
            })
            .to_string(),
        ];

        let parsed = parse_rollout_lines(lines.iter().map(String::as_str), 99).unwrap();

        assert!(!rollout_is_user_visible(&parsed));
    }

    #[test]
    fn project_scan_only_fully_parses_owned_rollouts() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("selected-project");
        let other_root = temp.path().join("other-project");
        let codex_home = temp.path().join("codex-home");
        let sessions = codex_home.join("sessions/2026/07/21");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&other_root).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        let project_root = fs::canonicalize(project_root).unwrap();
        let other_root = fs::canonicalize(other_root).unwrap();

        fs::write(
            sessions.join("owned.jsonl"),
            format!(
                "{}\n{}\n",
                json!({
                    "timestamp": 100,
                    "type": "session_meta",
                    "payload": {
                        "id": "019f742a-a206-7932-876c-9db8d8ce575a",
                        "cwd": project_root
                    }
                }),
                json!({
                    "timestamp": 101,
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": "Owned thread"}
                })
            ),
        )
        .unwrap();
        fs::write(
            sessions.join("other.jsonl"),
            format!(
                "{}\nthis trailing record is deliberately malformed\n",
                json!({
                    "timestamp": 200,
                    "type": "session_meta",
                    "payload": {
                        "id": "019f742a-a206-7932-876c-9db8d8ce575b",
                        "cwd": other_root
                    }
                })
            ),
        )
        .unwrap();

        let repository = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
        let mut warnings = Vec::new();
        let threads =
            scan_codex_threads(&repository, &codex_home, &project_root, true, &mut warnings)
                .unwrap();

        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].thread_id, "019f742a-a206-7932-876c-9db8d8ce575a");
        assert!(warnings.is_empty());
        let cache = load_chat_history_cache(
            repository.root(),
            &repository.root().join("chat_history_cache.json"),
            &mut warnings,
        );
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.ownership.len(), 2);
        assert!(cache
            .ownership
            .values()
            .all(|entry| entry.identity.thread_id.is_some()));
    }

    #[test]
    fn owned_rollout_lookup_parses_the_uuid_named_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let sessions = temp.path().join("codex/sessions/2026/07/21");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        let project_root = fs::canonicalize(project_root).unwrap();
        let thread_id = "019f742a-a206-7932-876c-9db8d8ce575a";
        let target = sessions.join(format!("rollout-2026-07-21T00-00-00-{thread_id}.jsonl"));
        fs::write(
            &target,
            format!(
                "{}\n{}\n",
                json!({"timestamp":100,"type":"session_meta","payload":{"id":thread_id,"cwd":project_root}}),
                json!({"timestamp":101,"type":"event_msg","payload":{"type":"agent_message","message":"Done"}}),
            ),
        )
        .unwrap();

        let found =
            find_owned_rollout(&temp.path().join("codex"), &project_root, thread_id).unwrap();

        assert_eq!(found, target);
    }

    #[test]
    fn owned_rollout_lookup_supports_renamed_files_with_bounded_identity_probe() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let sessions = temp.path().join("codex/archived_sessions");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        let project_root = fs::canonicalize(project_root).unwrap();
        let thread_id = "019f742a-a206-7932-876c-9db8d8ce575a";
        let target = sessions.join("renamed-session.jsonl");
        fs::write(
            &target,
            format!(
                "{}\n{}\n",
                json!({"timestamp":100,"type":"session_meta","payload":{"id":thread_id,"cwd":project_root}}),
                json!({"timestamp":101,"type":"event_msg","payload":{"type":"agent_message","message":"Done"}}),
            ),
        )
        .unwrap();

        let found =
            find_owned_rollout(&temp.path().join("codex"), &project_root, thread_id).unwrap();

        assert_eq!(found, target);
    }

    #[test]
    fn owned_rollout_lookup_rejects_a_spoofed_uuid_filename() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let sessions = temp.path().join("codex/sessions");
        fs::create_dir_all(&project_root).unwrap();
        fs::create_dir_all(&sessions).unwrap();
        let project_root = fs::canonicalize(project_root).unwrap();
        let requested_id = "019f742a-a206-7932-876c-9db8d8ce575a";
        let recorded_id = "019f742a-a206-7932-876c-9db8d8ce575b";
        let target = sessions.join(format!("rollout-2026-07-21T00-00-00-{requested_id}.jsonl"));
        fs::write(
            target,
            format!(
                "{}\n",
                json!({"timestamp":100,"type":"session_meta","payload":{"id":recorded_id,"cwd":project_root}}),
            ),
        )
        .unwrap();

        let error = find_owned_rollout(&temp.path().join("codex"), &project_root, requested_id)
            .unwrap_err();

        assert_eq!(
            error,
            "Codex thread does not belong to the selected project"
        );
    }

    #[test]
    fn rollout_stream_reads_past_256_records_and_uses_first_and_last_timestamps() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollout.jsonl");
        let mut body = String::new();
        body.push_str(&format!(
            "{}\n",
            json!({"timestamp":"2026-07-18T16:20:00Z","type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project","git":{"branch":"main"}}})
        ));
        body.push_str(&format!(
            "{}\n",
            json!({"timestamp":"2026-07-18T16:21:00Z","type":"event_msg","payload":{"type":"user_message","message":"Real user request"}})
        ));
        for index in 0..300 {
            body.push_str(&format!(
                "{}\n",
                json!({"timestamp":1_784_391_700u64 + index,"type":"event_msg","payload":{"type":"agent_reasoning","text":"internal"}})
            ));
        }
        body.push_str(&format!(
            "{}\n",
            json!({"timestamp":"2026-07-18T17:20:00Z","type":"event_msg","payload":{"type":"agent_message","message":"Visible response","phase":"final"}})
        ));
        fs::write(&path, body).unwrap();

        let mut warnings = Vec::new();
        let parsed = parse_rollout_file(&path, 123, &mut warnings).unwrap();

        assert_eq!(parsed.thread.started_at, 1_784_391_600);
        assert_eq!(parsed.thread.ended_at, 1_784_395_200);
        assert_eq!(parsed.thread.summary, "Real user request");
        assert_eq!(parsed.thread.user_round_count, 1);
        assert_eq!(parsed.thread.agent_message_count, 1);
        assert!(parsed.thread.metrics_complete);
        assert!(warnings.is_empty());
    }

    #[test]
    fn rollout_larger_than_16_mib_is_fully_streamed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large-rollout.jsonl");
        let mut writer = BufWriter::new(File::create(&path).unwrap());
        writeln!(writer, "{}", json!({"timestamp":100,"type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project"}})).unwrap();
        let padding = "x".repeat(900);
        for timestamp in 101..19_000 {
            writeln!(writer, "{}", json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"agent_reasoning","text":padding}})).unwrap();
        }
        writeln!(writer, "{}", json!({"timestamp":20_000,"type":"event_msg","payload":{"type":"agent_message","message":"visible final response"}})).unwrap();
        writer.flush().unwrap();
        assert!(fs::metadata(&path).unwrap().len() > 16 * 1024 * 1024);

        let mut warnings = Vec::new();
        let parsed = parse_rollout_file(&path, 1, &mut warnings).unwrap();
        assert_eq!(parsed.thread.started_at, 100);
        assert_eq!(parsed.thread.ended_at, 20_000);
        assert_eq!(parsed.thread.agent_message_count, 1);
        assert!(warnings.is_empty());
    }

    #[test]
    fn oversized_image_records_recover_visible_user_and_assistant_text() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("image-rollout.jsonl");
        let image = "a".repeat(MAX_LINE_BYTES + 1);
        let lines = [
            json!({"timestamp":100,"type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project"}}),
            json!({"timestamp":101,"type":"event_msg","payload":{"type":"user_message","message":[{"type":"input_text","text":"Visible user request"},{"type":"input_image","image_url":image}]}}),
            json!({"timestamp":102,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Visible Codex response"},{"type":"output_image","image_url":image}]}}),
        ];
        fs::write(
            &path,
            lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();

        let mut reader = BufReader::new(File::open(&path).unwrap());
        let mut bytes = Vec::new();
        assert_eq!(
            read_bounded_line(&mut reader, &mut bytes, MAX_LINE_BYTES).unwrap(),
            BoundedLine::Line
        );
        let record_start = reader.stream_position().unwrap();
        assert_eq!(
            read_bounded_line(&mut reader, &mut bytes, MAX_LINE_BYTES).unwrap(),
            BoundedLine::Oversized
        );
        let record_end = reader.stream_position().unwrap();
        let projection = recover_oversized_record(&mut reader, record_start, record_end).unwrap();
        let projected_bytes = serde_json::to_vec(&projection).unwrap();
        assert!(projected_bytes.len() < 1_024);
        assert!(!projected_bytes
            .windows(9)
            .any(|bytes| bytes == b"image_url"));

        let mut warnings = Vec::new();
        let parsed = parse_rollout_file(&path, 1, &mut warnings).unwrap();
        assert_eq!(parsed.thread.summary, "Visible user request");
        assert_eq!(parsed.thread.user_round_count, 1);
        assert!(parsed.thread.metrics_complete);
        assert!(warnings.is_empty());

        let first_page = parse_thread_detail_file(&path, None, 1).unwrap();
        assert_eq!(first_page.turns.len(), 1);
        assert_eq!(first_page.turns[0].ordinal, 1);
        assert_eq!(first_page.turns[0].preview, "Visible Codex response");
        assert_eq!(first_page.next_cursor, Some(1));

        let second_page = parse_thread_detail_file(&path, Some(1), 1).unwrap();
        assert_eq!(second_page.turns.len(), 1);
        assert_eq!(second_page.turns[0].ordinal, 0);
        assert_eq!(second_page.turns[0].preview, "Visible user request");
        assert_eq!(second_page.next_cursor, None);
    }

    #[test]
    fn malformed_oversized_records_warn_once_and_continue() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("malformed-oversized-rollout.jsonl");
        let malformed = format!(
            "{{\"timestamp\":101,\"payload\":{}",
            "x".repeat(MAX_LINE_BYTES + 1)
        );
        fs::write(
            &path,
            format!(
                "{}\n{malformed}\n{malformed}\n{}\n",
                json!({"timestamp":100,"type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project"}}),
                json!({"timestamp":103,"type":"event_msg","payload":{"type":"agent_message","message":"Later valid response"}}),
            ),
        )
        .unwrap();

        let mut warnings = Vec::new();
        let parsed = parse_rollout_file(&path, 1, &mut warnings).unwrap();
        assert_eq!(parsed.thread.ended_at, 103);
        assert_eq!(parsed.thread.agent_message_count, 1);
        assert!(!parsed.thread.metrics_complete);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("malformed oversized JSONL record"));
    }

    #[test]
    fn oversized_irrelevant_bodies_are_silent_but_keep_metrics() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("oversized-metrics-rollout.jsonl");
        let large = "x".repeat(MAX_LINE_BYTES + 1);
        let lines = [
            json!({"timestamp":100,"type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project"}}),
            json!({"timestamp":101,"type":"compacted","replacement_history":large}),
            json!({"timestamp":102,"type":"response_item","payload":{"type":"custom_tool_call","name":"large_tool","arguments":large}}),
            json!({"timestamp":103,"type":"response_item","payload":{"type":"custom_tool_call_output","output":large}}),
            json!({"timestamp":104,"type":"event_msg","payload":{"type":"token_count","padding":large,"info":{"total_token_usage":{"total_tokens":42}}}}),
            json!({"timestamp":105,"type":"event_msg","payload":{"type":"agent_message","message":"Done"}}),
        ];
        fs::write(
            &path,
            lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();

        let mut warnings = Vec::new();
        let parsed = parse_rollout_file(&path, 1, &mut warnings).unwrap();
        assert_eq!(parsed.thread.ended_at, 105);
        assert_eq!(parsed.thread.tool_call_count, 1);
        assert_eq!(parsed.thread.total_tokens, Some(42));
        assert!(parsed.thread.metrics_complete);
        assert!(warnings.is_empty());

        let details = parse_thread_detail_file(&path, None, 50).unwrap();
        assert_eq!(details.turns.len(), 1);
        assert_eq!(details.turns[0].preview, "Done");
    }

    #[test]
    fn session_windows_are_start_inclusive_and_end_exclusive() {
        assert!(thread_in_window(&thread("start", 1, 100), 100, 200));
        assert!(thread_in_window(&thread("inside", 1, 199), 100, 200));
        assert!(!thread_in_window(&thread("end", 1, 200), 100, 200));
        assert!(!thread_in_window(&thread("older", 1, 99), 100, 200));
    }

    #[test]
    fn metadata_cache_excludes_chat_text_and_invalidates_changed_files() {
        let parsed = parse_rollout_lines(
            [
                r#"{"timestamp":100,"type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project"}}"#,
                r#"{"timestamp":101,"type":"event_msg","payload":{"type":"user_message","message":"private prompt text"}}"#,
            ],
            99,
        )
        .unwrap();
        let cached = RolloutCacheEntry {
            size: 200,
            modified_nanos: 300,
            parsed: cache_safe_rollout(parsed),
        };

        assert!(cached.parsed.thread.title.is_empty());
        assert!(cached.parsed.thread.summary.is_empty());
        assert!(cached_rollout(&cached, 200, 300, true).is_some());
        assert!(cached_rollout(&cached, 201, 300, true).is_none());
        assert!(cached_rollout(&cached, 200, 301, true).is_none());
        assert!(cached_rollout(&cached, 200, 300, false).is_none());
    }

    #[test]
    fn metadata_cache_uses_private_bounded_atomic_persistence() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
        let path = repository.root().join("chat_history_cache.json");
        let mut warnings = Vec::new();
        save_chat_history_cache(
            repository.root(),
            &path,
            &ChatHistoryCache::default(),
            &mut warnings,
        );
        assert!(warnings.is_empty());
        assert_eq!(
            load_chat_history_cache(repository.root(), &path, &mut warnings).schema,
            CHAT_CACHE_SCHEMA
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(repository.root())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn chat_cache_uses_the_current_reset_schema() {
        assert_eq!(CHAT_CACHE_SCHEMA, 1);
    }

    #[test]
    fn duplicate_rollouts_prefer_complete_then_latest_metadata_and_details() {
        let complete = parse_rollout_lines(
            [
                r#"{"timestamp":100,"type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project"}}"#,
                r#"{"timestamp":200,"type":"event_msg","payload":{"type":"agent_message","message":"complete"}}"#,
            ],
            1,
        )
        .unwrap();
        let partial = parse_rollout_lines(
            [
                r#"{"timestamp":100,"type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project"}}"#,
                "malformed",
                r#"{"timestamp":300,"type":"event_msg","payload":{"type":"agent_message","message":"newer but partial"}}"#,
            ],
            1,
        )
        .unwrap();
        assert!(rollout_is_preferred(&complete, &partial));
        assert!(!rollout_is_preferred(&partial, &complete));
    }

    #[test]
    fn rollout_metrics_ignore_injected_user_roles_and_use_reported_token_maximum() {
        let lines = vec![
            json!({"timestamp":100,"type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project"}}).to_string(),
            json!({"timestamp":101,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<recommended_plugins>not a user turn</recommended_plugins>"}]}}).to_string(),
            json!({"timestamp":101,"type":"event_msg","payload":{"type":"user_message","message":"<environment_context>also injected</environment_context>"}}).to_string(),
            json!({"timestamp":102,"type":"event_msg","payload":{"type":"user_message","message":"Actual prompt"}}).to_string(),
            json!({"timestamp":103,"type":"event_msg","payload":{"type":"agent_message","message":"Answer","phase":"final"}}).to_string(),
            json!({"timestamp":104,"type":"response_item","payload":{"type":"function_call","name":"exec_command"}}).to_string(),
            json!({"timestamp":105,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":1200}}}}).to_string(),
            json!({"timestamp":106,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":900}}}}).to_string(),
        ];
        let parsed = parse_rollout_lines(lines.iter().map(String::as_str), 999).unwrap();

        assert_eq!(parsed.thread.summary, "Actual prompt");
        assert_eq!(parsed.thread.user_round_count, 1);
        assert_eq!(parsed.thread.agent_message_count, 1);
        assert_eq!(parsed.thread.tool_call_count, 1);
        assert_eq!(parsed.thread.total_tokens, Some(1200));
        assert_eq!(parsed.thread.started_at, 100);
        assert_eq!(parsed.thread.ended_at, 106);
    }

    #[test]
    fn chat_detail_pages_only_visible_user_and_agent_messages() {
        let lines = vec![
            json!({"timestamp":100,"type":"event_msg","payload":{"type":"user_message","message":"First user message"}}).to_string(),
            json!({"timestamp":101,"type":"event_msg","payload":{"type":"agent_reasoning","text":"hidden"}}).to_string(),
            json!({"timestamp":102,"type":"event_msg","payload":{"type":"agent_message","message":"First agent answer","phase":"final"}}).to_string(),
            json!({"timestamp":103,"type":"event_msg","payload":{"type":"user_message","message":"Second user message"}}).to_string(),
        ];

        let page = parse_thread_detail_lines(lines.iter().map(String::as_str), None, 2);
        assert_eq!(page.turns.len(), 2);
        assert_eq!(page.turns[0].role, ChatTurnRole::Assistant);
        assert_eq!(page.turns[1].role, ChatTurnRole::User);
        assert_eq!(page.next_cursor, Some(2));

        let older = parse_thread_detail_lines(lines.iter().map(String::as_str), Some(2), 2);
        assert_eq!(older.turns.len(), 1);
        assert_eq!(older.turns[0].role, ChatTurnRole::User);
        assert_eq!(older.next_cursor, None);
    }

    #[test]
    fn chat_detail_pages_start_with_the_latest_ten_turns() {
        let lines = (0..25)
            .map(|index| {
                json!({
                    "timestamp": 100 + index,
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": format!("Message {index}")}
                })
                .to_string()
            })
            .collect::<Vec<_>>();

        let latest = parse_thread_detail_lines(lines.iter().map(String::as_str), None, 10);
        assert_eq!(latest.turns.len(), 10);
        assert_eq!(latest.turns.first().unwrap().ordinal, 15);
        assert_eq!(latest.turns.last().unwrap().ordinal, 24);
        assert_eq!(latest.next_cursor, Some(10));

        let middle = parse_thread_detail_lines(lines.iter().map(String::as_str), Some(10), 10);
        assert_eq!(middle.turns.len(), 10);
        assert_eq!(middle.turns.first().unwrap().ordinal, 5);
        assert_eq!(middle.turns.last().unwrap().ordinal, 14);
        assert_eq!(middle.next_cursor, Some(20));

        let oldest = parse_thread_detail_lines(lines.iter().map(String::as_str), Some(20), 10);
        assert_eq!(oldest.turns.len(), 5);
        assert_eq!(oldest.turns.first().unwrap().ordinal, 0);
        assert_eq!(oldest.turns.last().unwrap().ordinal, 4);
        assert_eq!(oldest.next_cursor, None);
    }

    #[test]
    fn streamed_chat_detail_pages_match_in_memory_pagination() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollout.jsonl");
        let lines = (0..25)
            .map(|index| {
                json!({
                    "timestamp": 100 + index,
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": format!("Message {index}")}
                })
                .to_string()
            })
            .collect::<Vec<_>>();
        fs::write(&path, lines.join("\n") + "\n").unwrap();

        for cursor in [None, Some(1), Some(10), Some(20), Some(25), Some(100)] {
            let expected = parse_thread_detail_lines(lines.iter().map(String::as_str), cursor, 7);
            let actual = parse_thread_detail_file(&path, cursor, 7).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn chat_details_include_assistant_response_items_without_duplicate_event_copies() {
        let lines = vec![
            json!({"timestamp":100,"type":"event_msg","payload":{"type":"agent_message","message":"Same answer"}}).to_string(),
            json!({"timestamp":100,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Same answer"}]}}).to_string(),
            json!({"timestamp":101,"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Response-only answer"}]}}).to_string(),
            json!({"timestamp":102,"type":"event_msg","payload":{"type":"user_message","message":"<permissions instructions>injected</permissions instructions>"}}).to_string(),
        ];

        let page = parse_thread_detail_lines(lines.iter().map(String::as_str), None, 50);
        assert_eq!(page.turns.len(), 2);
        assert_eq!(page.turns[0].preview, "Same answer");
        assert_eq!(page.turns[1].preview, "Response-only answer");
        assert!(page
            .turns
            .iter()
            .all(|turn| turn.role == ChatTurnRole::Assistant));
    }

    #[test]
    fn malformed_metrics_do_not_override_valid_rollout_endpoints_with_index_time() {
        let lines = vec![
            json!({"timestamp":100,"type":"session_meta","payload":{"id":"019f742a-a206-7932-876c-9db8d8ce575a","cwd":"/tmp/project"}}).to_string(),
            "malformed".to_string(),
            json!({"timestamp":200,"type":"event_msg","payload":{"type":"agent_message","message":"done"}}).to_string(),
        ];
        let parsed = parse_rollout_lines(lines.iter().map(String::as_str), 999).unwrap();
        assert!(!parsed.thread.metrics_complete);
        assert!(parsed.has_record_endpoints);
        let mut thread = parsed.thread;
        apply_index_metadata(
            &mut thread,
            true,
            &SessionIndexEntry {
                title: None,
                updated_at: Some(10_000),
            },
        );
        assert_eq!(thread.ended_at, 200);
    }

    #[test]
    fn bounded_line_reader_discards_oversized_unterminated_content() {
        let mut input = std::io::Cursor::new(format!("{}\n{{}}\n", "x".repeat(200)));
        let mut output = Vec::new();
        assert_eq!(
            read_bounded_line(&mut input, &mut output, 32).unwrap(),
            BoundedLine::Oversized
        );
        assert!(output.capacity() <= 32);
        assert_eq!(
            read_bounded_line(&mut input, &mut output, 32).unwrap(),
            BoundedLine::Line
        );
        assert_eq!(output, b"{}");
    }

    #[test]
    fn rollout_uses_mtime_fallback_and_truncates_unicode_summary() {
        let long_message = "界".repeat(MAX_SUMMARY_CHARS + 20);
        let lines = vec![
            json!({"type":"session_meta","payload":{"session_id":"019f742a-a206-7932-876c-9db8d8ce575a","working_directory":"/tmp/project"}}).to_string(),
            json!({"payload":{"role":"user","content":long_message}}).to_string(),
        ];
        let parsed = parse_rollout_lines(lines.iter().map(String::as_str), 456).unwrap();

        assert_eq!(parsed.thread.started_at, 456);
        assert_eq!(parsed.thread.ended_at, 456);
        assert_eq!(parsed.thread.summary.chars().count(), MAX_SUMMARY_CHARS);
        assert!(parsed.thread.summary.ends_with('…'));
    }

    #[test]
    fn index_title_and_time_override_rollout_fallbacks() {
        let index = parse_session_index_lines([
            r#"{"id":"019f742a-a206-7932-876c-9db8d8ce575a","thread_name":"First","updated_at":100}"#,
            r#"{"thread_id":"019f742a-a206-7932-876c-9db8d8ce575a","title":"Renamed","updatedAt":"2026-07-18T16:20:00Z"}"#,
        ]);
        let item = index.get("019f742a-a206-7932-876c-9db8d8ce575a").unwrap();
        assert_eq!(item.title.as_deref(), Some("Renamed"));
        assert_eq!(item.updated_at, Some(1_784_391_600));

        let long_title = "界".repeat(MAX_SUMMARY_CHARS + 20);
        let line = json!({"id":"thread-long","title":long_title}).to_string();
        let truncated = parse_session_index_lines([line.as_str()]);
        let title = truncated["thread-long"].title.as_deref().unwrap();
        assert_eq!(title.chars().count(), MAX_SUMMARY_CHARS);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn portable_index_title_parser_skips_invalid_rows_and_normalizes_titles() {
        let bytes = b"not json\n\
            {\"id\":\"thread-a\",\"thread_name\":\"  Improve   cloud titles  \"}\n\
            \xff\n\
            {\"thread_id\":\"thread-b\",\"title\":\"Second title\"}\n";
        let titles = parse_session_index_titles(bytes);

        assert_eq!(
            titles,
            BTreeMap::from([
                ("thread-a".to_string(), "Improve cloud titles".to_string()),
                ("thread-b".to_string(), "Second title".to_string()),
            ])
        );
    }

    #[test]
    fn oversized_or_malformed_lines_become_partial_warnings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session_index.jsonl");
        fs::write(
            &path,
            format!("not json\n{{\"id\":\"{}\"}}\n", "x".repeat(MAX_LINE_BYTES)),
        )
        .unwrap();
        let mut warnings = Vec::new();
        let index = read_session_index(&path, &mut warnings).unwrap();
        assert!(index.is_empty());
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn ownership_uses_canonical_path_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let child = project.join("packages/app");
        let sibling = temp.path().join("project-copy");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        let canonical = fs::canonicalize(&project).unwrap();

        assert!(cwd_belongs_to_project(&child, &canonical));
        assert!(!cwd_belongs_to_project(&sibling, &canonical));
        assert!(!cwd_belongs_to_project(Path::new("../project"), &canonical));
    }

    #[test]
    fn git_history_is_first_parent_and_cursor_is_validated() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test"]);
        git(root, &["config", "user.email", "test@example.com"]);
        for (name, value) in [("first", "1"), ("second", "2"), ("third", "3")] {
            fs::write(root.join("file"), value).unwrap();
            git(root, &["add", "file"]);
            git(root, &["commit", "-m", name]);
        }
        let discovery = discover_git_history(root, None, None, 2).unwrap().unwrap();
        assert_eq!(discovery.commits.len(), 2);
        assert!(discovery.next_cursor.is_some());
        let next = discover_git_history(root, Some("main"), discovery.next_cursor.as_deref(), 2)
            .unwrap()
            .unwrap();
        assert_eq!(next.commits.len(), 1);
        assert!(discover_git_history(root, Some("main; touch /tmp/nope"), None, 2).is_err());
        assert!(discover_git_history(root, Some("main"), Some("not-a-sha"), 2).is_err());
    }

    #[test]
    fn git_history_allows_an_unborn_current_branch() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "-b", "main"]);

        let discovery = discover_git_history(root, None, None, 50).unwrap().unwrap();
        assert_eq!(discovery.selected_branch, "main");
        assert!(!discovery.selected_ref_available);
        assert!(discovery.commits.is_empty());
        assert!(discovery.next_cursor.is_none());
        assert_eq!(
            discovery
                .branches
                .iter()
                .find(|branch| branch.name == "main"),
            Some(&GitBranchSummary {
                name: "main".to_string(),
                is_current: true,
                available: true,
            })
        );

        let requested = discover_git_history(root, Some("main"), None, 50)
            .unwrap()
            .unwrap();
        assert!(requested.commits.is_empty());
    }

    #[test]
    fn github_remote_urls_are_canonicalized_without_credentials() {
        for (remote, expected) in [
            (
                "https://github.com/Owner/repository.git",
                Some("https://github.com/Owner/repository"),
            ),
            (
                "ssh://git@github.com/Owner/repository.git",
                Some("https://github.com/Owner/repository"),
            ),
            (
                "git@github.com:Owner/repository.git",
                Some("https://github.com/Owner/repository"),
            ),
            (
                "https://user:secret@github.com/Owner/repository.git",
                Some("https://github.com/Owner/repository"),
            ),
            ("https://github.com.evil/Owner/repository.git", None),
            ("https://github.com/Owner/repository/tree/main", None),
            ("file:///tmp/Owner/repository.git", None),
        ] {
            assert_eq!(normalize_github_repository_url(remote).as_deref(), expected);
        }
    }

    #[test]
    fn github_remote_discovery_prefers_origin_then_another_github_remote() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "-b", "main"]);
        git(
            root,
            &[
                "remote",
                "add",
                "mirror",
                "git@github.com:mirror/repository.git",
            ],
        );
        assert_eq!(
            discover_github_repository_url(root).as_deref(),
            Some("https://github.com/mirror/repository")
        );

        git(
            root,
            &[
                "remote",
                "add",
                "origin",
                "https://user:secret@github.com/owner/repository.git",
            ],
        );
        assert_eq!(
            discover_github_repository_url(root).as_deref(),
            Some("https://github.com/owner/repository")
        );

        git(
            root,
            &[
                "remote",
                "set-url",
                "origin",
                "https://gitlab.com/owner/repository.git",
            ],
        );
        assert_eq!(
            discover_github_repository_url(root).as_deref(),
            Some("https://github.com/mirror/repository")
        );
    }

    #[test]
    fn recorded_deleted_branch_remains_selectable_but_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "Test"]);
        git(root, &["config", "user.email", "test@example.com"]);
        fs::write(root.join("file"), "one").unwrap();
        git(root, &["add", "file"]);
        git(root, &["commit", "-m", "first"]);
        let recorded = BTreeSet::from(["deleted/history".to_string()]);

        let discovery = discover_git_history_with_recorded_branches(
            root,
            Some("deleted/history"),
            None,
            50,
            &recorded,
        )
        .unwrap()
        .unwrap();
        assert!(!discovery.selected_ref_available);
        assert!(discovery.commits.is_empty());
        assert_eq!(
            discovery
                .branches
                .iter()
                .find(|branch| branch.name == "deleted/history")
                .map(|branch| branch.available),
            Some(false)
        );
    }

    #[test]
    fn active_session_detection_rejects_large_clock_skew() {
        assert!(session_mtime_is_active(1_000, 900));
        assert!(session_mtime_is_active(1_000, 1_100));
        assert!(!session_mtime_is_active(1_000, 699));
        assert!(!session_mtime_is_active(1_000, 1_301));
    }

    #[test]
    fn terminal_command_is_shell_safe_and_uuid_is_strict() {
        let id = "019f742a-a206-7932-876c-9db8d8ce575a";
        let command = terminal_resume_command(
            Path::new("/opt/homebrew/bin/codex"),
            id,
            Path::new("/tmp/client's project"),
            Path::new("/tmp/client's config/.codex"),
        )
        .unwrap();
        assert_eq!(
            command,
            "CODEX_HOME='/tmp/client'\"'\"'s config/.codex' '/opt/homebrew/bin/codex' resume '019f742a-a206-7932-876c-9db8d8ce575a' -C '/tmp/client'\"'\"'s project'"
        );
        assert!(terminal_resume_command(
            Path::new("/opt/homebrew/bin/codex"),
            "bad; open -a Calculator",
            Path::new("/tmp/p"),
            Path::new("/tmp/.codex"),
        )
        .is_err());
        assert!(terminal_resume_command(
            Path::new("codex"),
            id,
            Path::new("/tmp/p"),
            Path::new("/tmp/.codex"),
        )
        .is_err());
    }

    #[test]
    fn app_commands_open_project_then_thread_in_pinned_codex_home() {
        let commands = codex_app_commands(
            "019f742a-a206-7932-876c-9db8d8ce575a",
            Path::new("/tmp/client's project"),
            Path::new("/tmp/client's config/.codex"),
        )
        .unwrap();
        assert_eq!(
            commands,
            [
                "'/usr/bin/open' -n -a '/Applications/ChatGPT.app' --env 'CODEX_HOME=/tmp/client'\"'\"'s config/.codex' 'codex://threads/new?path=%2Ftmp%2Fclient%27s%20project'".to_string(),
                "'/usr/bin/open' -n -a '/Applications/ChatGPT.app' --env 'CODEX_HOME=/tmp/client'\"'\"'s config/.codex' 'codex://threads/019f742a-a206-7932-876c-9db8d8ce575a'".to_string(),
            ]
        );
        assert!(codex_app_commands(
            "019f742a-a206-7932-876c-9db8d8ce575a",
            Path::new("relative-project"),
            Path::new("/tmp/.codex"),
        )
        .is_err());
    }

    fn git(root: &Path, args: &[&str]) {
        let result = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
