//! Provider-neutral project Activity: local session discovery for every
//! bound provider, verified remote-session metadata for every provider scope
//! (bound or not), Git mapping over normalized evidence, and safe session
//! actions dispatched through the registered adapters.
//!
//! Activity is recomputed on demand. Push also captures bounded Git metadata
//! for the portable manifest. Viewing a remote
//! session never advances any scope base and never writes to a provider
//! home.

use crate::persistence::{
    read_json_bounded, write_json_atomic, LocalProjectActivityStateSnapshot, LocalStateRepository,
};
use crate::provider_capture::{modified_secs, LocalSessionView};
use crate::providers::{
    AgentProviderRegistry, SessionActionContext, SessionActionDescriptor, SessionActionRequest,
    SessionDiscoveryContext, SessionFileSummary,
};
use crate::storage_core::ConfiguredObjectStore;
use mallard_domain::{
    cloud_conversation_resource_id, conversation_resource_id, initial_conversation_thread_id,
    now_secs, parse_conversation_resource_id, retained_conversation_thread_id, ActionStatus,
    BindingState, BundleSnapshot, CloudThreadId, ConversationBindings,
    ConversationMaterializationState, LocalProjectId, LocalProviderProfileId, MachineProjectState,
    MaterializationStatus, Materializations, PreservedProvenance, ProviderKey, StorageId,
    SyncConfig, SyncScopeKey, ThreadId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::UNIX_EPOCH;

const DEFAULT_HISTORY_WINDOW_DAYS: u64 = 7;
const DAY_SECS: u64 = 24 * 60 * 60;
const MAX_GIT_COMMITS: usize = mallard_domain::MAX_GIT_HISTORY_COMMITS;
const ACTIVE_WINDOW_SECS: u64 = 300;
const ACTIVITY_CACHE_SCHEMA: u32 = 1;
const MAX_ACTIVITY_CACHE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ACTIVITY_CACHE_ENTRIES: usize = 50_000;

/// How one provider participates in this project's Activity.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccessState {
    /// Bound with a readable home: full local discovery and actions.
    BoundReady,
    /// Bound but the profile is currently unreadable; rows fail closed.
    BoundUnavailable,
    /// Present remotely, not bound locally: rows are visible, never
    /// selectable or actionable.
    ViewOnly,
    /// Present remotely with no registered adapter: generic metadata only.
    Unsupported,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProviderActivityState {
    pub provider: ProviderKey,
    pub display_name: String,
    pub access: ProviderAccessState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionSourceState {
    LocalOnly,
    RemoteOnly,
    LocalAndRemote,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectSessionSummary {
    /// The frontend key and selection identity. Different providers may use
    /// the same native session ID, so this is never the native ID alone.
    pub resource_id: String,
    pub provider: ProviderKey,
    pub provider_display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_thread_id: Option<String>,
    pub source: SessionSourceState,
    pub access: ProviderAccessState,
    pub title: String,
    /// Present when this logical thread is the retained local side of a
    /// divergent keep-both Pull — from the local binding for bound rows, or
    /// from the published thread record for remote-only rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved: Option<PreservedProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub started_at: u64,
    pub ended_at: u64,
    /// The last human input, which places the session on one commit. A
    /// storage-only row carries the time recorded at Push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_at: Option<u64>,
    /// Machine-local provider transcript selected by discovery. This is
    /// returned only for a locally present session and never enters a
    /// portable manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    /// Machine-local reason this visible session cannot participate in sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_sha: Option<String>,
    pub is_active: bool,
    pub user_round_count: u64,
    pub agent_message_count: u64,
    pub tool_call_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    pub metrics_complete: bool,
    #[serde(default)]
    pub actions: Vec<SessionActionDescriptor>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CommitSessionReference {
    pub resource_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GitCommitSummary {
    pub sha: String,
    pub short_sha: String,
    pub committed_at: u64,
    pub subject: String,
    #[serde(default)]
    pub session_refs: Vec<CommitSessionReference>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GitBranchSummary {
    pub name: String,
    pub is_current: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GitHistoryPage {
    pub selected_branch: String,
    pub branches: Vec<GitBranchSummary>,
    pub commits: Vec<GitCommitSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_repository_url: Option<String>,
    pub unique_session_count: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UnmappedSessionReference {
    pub resource_id: String,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageSyncSummary {
    pub storage_id: String,
    pub storage_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pull_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_push_at: Option<u64>,
}

/// What lies before `window_start`, so the page can name it instead of
/// offering a blind step backwards.
///
/// Every field is read off data the response already holds: `all_commits` is
/// loaded by count rather than by date, and `all_sessions` is the full list the
/// window filters. Nothing here asks git a second question.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct OlderActivity {
    /// The newest thing older than the window. The next page ends just past
    /// this, so a step backwards always lands on data instead of on whatever
    /// happens to be thirty days earlier.
    pub newest_at: u64,
    /// Commits older than the window that are not already on the page. A
    /// commit outside the window still renders when a visible session maps to
    /// it, and counting those again would promise rows that are already here.
    pub commits: usize,
    pub sessions: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectActivity {
    pub project_id: String,
    pub sessions: Vec<ProjectSessionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitHistoryPage>,
    #[serde(default)]
    pub unmapped: Vec<UnmappedSessionReference>,
    #[serde(default)]
    pub providers: Vec<ProviderActivityState>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub window_start: u64,
    pub window_end: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older: Option<OlderActivity>,
    #[serde(default)]
    pub storage_sync: Vec<StorageSyncSummary>,
}

// ---------------------------------------------------------------------------
// Project resolution
// ---------------------------------------------------------------------------

pub struct ResolvedProviderHome {
    pub profile_id: mallard_domain::LocalProviderProfileId,
    pub home: PathBuf,
}

pub struct ResolvedActivityProject {
    pub project_id: LocalProjectId,
    pub bundle_id: mallard_domain::BundleId,
    pub project_root: PathBuf,
    pub binding: mallard_domain::ProjectBinding,
    /// Bound providers with a currently readable canonical home.
    pub homes: BTreeMap<ProviderKey, ResolvedProviderHome>,
    /// Bound providers whose profile failed to resolve, with the reason.
    /// These rows fail closed without blocking other providers.
    pub unavailable: BTreeMap<ProviderKey, String>,
}

/// Resolve the project, binding, and every bound provider home. A provider
/// whose profile is missing or moved lands in `unavailable` instead of
/// failing the whole resolution: an unreadable Claude profile must not
/// prevent safe Codex activity, and the reverse holds too.
pub fn resolve_activity_project(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<ResolvedActivityProject, String> {
    let state = repository.load_project_state_snapshot()?;
    resolve_activity_project_from_state(&state.config, &state.bindings, local_project_id)
}

/// Resolve one Activity project from a coherent, already-validated machine
/// state snapshot.
pub fn resolve_activity_project_from_state(
    config: &SyncConfig,
    state: &MachineProjectState,
    local_project_id: &LocalProjectId,
) -> Result<ResolvedActivityProject, String> {
    let project = config.project(local_project_id).ok_or_else(|| {
        "project is not registered; unfinished setup drafts have no history".to_string()
    })?;
    let binding = state
        .bindings
        .iter()
        .find(|binding| {
            &binding.local_project_id == local_project_id && binding.state == BindingState::Active
        })
        .ok_or_else(|| "project has no active binding on this machine".to_string())?;

    let project_root = fs::canonicalize(&binding.project_root)
        .map_err(|error| format!("resolve project root '{}': {error}", binding.project_root))?;
    if project_root.as_path() != Path::new(&binding.canonical_project_root) {
        return Err(
            "project root changed since the binding was saved; open Project Settings".to_string(),
        );
    }
    let mut homes = BTreeMap::new();
    let mut unavailable = BTreeMap::new();
    for (provider, profile_id) in &binding.profile_ids {
        let resolved = state
            .profiles
            .iter()
            .find(|profile| &profile.profile_id == profile_id && &profile.provider == provider)
            .ok_or_else(|| {
                format!(
                    "project's {} profile is missing; open Project Settings",
                    provider
                )
            })
            .and_then(|profile| {
                let home = fs::canonicalize(&profile.path).map_err(|error| {
                    format!("resolve {} profile '{}': {error}", provider, profile.path)
                })?;
                if home.as_path() != Path::new(&profile.canonical_path) {
                    return Err(format!(
                        "{} profile path changed; open Project Settings",
                        provider
                    ));
                }
                Ok(ResolvedProviderHome {
                    profile_id: profile.profile_id.clone(),
                    home,
                })
            });
        match resolved {
            Ok(home) => {
                homes.insert(provider.clone(), home);
            }
            Err(reason) => {
                unavailable.insert(provider.clone(), reason);
            }
        }
    }
    Ok(ResolvedActivityProject {
        project_id: local_project_id.clone(),
        bundle_id: project.bundle_id.clone(),
        project_root,
        binding: binding.clone(),
        homes,
        unavailable,
    })
}

// ---------------------------------------------------------------------------
// Local discovery with the activity cache
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ActivityCacheEntry {
    size: u64,
    modified_nanos: u64,
    summary: SessionFileSummary,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct ActivityCache {
    schema: u32,
    #[serde(default)]
    entries: BTreeMap<String, ActivityCacheEntry>,
}

/// One locally discovered provider session with its bounded summary.
#[derive(Clone, Debug)]
pub struct LocalSessionRecord {
    pub provider: ProviderKey,
    pub session_id: String,
    pub physical_path: PathBuf,
    pub display_name: String,
    pub summary: SessionFileSummary,
    pub is_active: bool,
}

fn modified_nanos(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0)
}

fn load_activity_cache(
    root: &Path,
    path: &Path,
    warnings: &mut Vec<String>,
) -> (ActivityCache, bool) {
    let empty_cache = || ActivityCache {
        schema: ACTIVITY_CACHE_SCHEMA,
        entries: BTreeMap::new(),
    };
    match read_json_bounded::<ActivityCache>(root, path, MAX_ACTIVITY_CACHE_BYTES) {
        Ok(Some(cache))
            if cache.schema == ACTIVITY_CACHE_SCHEMA
                && cache.entries.len() <= MAX_ACTIVITY_CACHE_ENTRIES =>
        {
            (cache, false)
        }
        Ok(Some(_)) => (empty_cache(), true),
        Ok(None) => (empty_cache(), false),
        Err(error) => {
            warnings.push(format!(
                "Ignored unreadable local activity cache '{}': {error}",
                path.display()
            ));
            (empty_cache(), true)
        }
    }
}

/// Summarize the session views from one already-completed provider discovery.
/// Activity and sync comparison use this entry point so their projections
/// share the exact same ownership witness and physical source selection.
pub fn scan_local_session_views(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    resolved: &ResolvedActivityProject,
    views: &[LocalSessionView],
    warnings: &mut Vec<String>,
) -> Result<Vec<LocalSessionRecord>, String> {
    let cache_path = repository.root().join("activity_cache.json");
    let (mut cache, mut cache_changed) =
        load_activity_cache(repository.root(), &cache_path, warnings);
    let now = now_secs();
    let mut records = Vec::new();
    let mut seen_cache_keys = BTreeSet::new();

    for (provider, home) in &resolved.homes {
        let Some(adapter) = registry.get(provider) else {
            warnings.push(format!(
                "provider '{}' is bound but has no registered adapter",
                provider
            ));
            continue;
        };
        let mut by_session: BTreeMap<String, LocalSessionRecord> = BTreeMap::new();
        for view in views.iter().filter(|view| &view.provider == provider) {
            if let Some(reason) = &view.descriptor.blocked_reason {
                warnings.push(format!(
                    "Skipped '{}': {}",
                    view.source_path.display(),
                    reason
                ));
                continue;
            }
            let metadata = match fs::metadata(&view.source_path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    warnings.push(format!(
                        "Cannot read '{}': {error}",
                        view.source_path.display()
                    ));
                    continue;
                }
            };
            let cache_key = format!(
                "{}\u{0}{}\u{0}{}\u{0}{}",
                resolved.project_id,
                provider,
                home.home.display(),
                view.source_path.display()
            );
            seen_cache_keys.insert(cache_key.clone());
            let file_nanos = modified_nanos(&metadata);
            let cached = cache
                .entries
                .get(&cache_key)
                .filter(|entry| entry.size == metadata.len() && entry.modified_nanos == file_nanos)
                .map(|entry| entry.summary.clone());
            let summary = match cached {
                Some(summary) => summary,
                None => match adapter.summarize_file(&view.source_path) {
                    Ok(summary) => {
                        cache.entries.insert(
                            cache_key,
                            ActivityCacheEntry {
                                size: metadata.len(),
                                modified_nanos: file_nanos,
                                summary: summary.clone(),
                            },
                        );
                        cache_changed = true;
                        summary
                    }
                    Err(error) => {
                        warnings.push(format!("Skipped '{}': {error}", view.source_path.display()));
                        continue;
                    }
                },
            };
            if summary.is_internal_subagent || summary.session_id != view.provider_thread_id {
                warnings.push(format!(
                    "Skipped '{}': the summarized session identity changed",
                    view.source_path.display()
                ));
                continue;
            }
            let record = LocalSessionRecord {
                provider: provider.clone(),
                session_id: view.provider_thread_id.clone(),
                physical_path: view.source_path.clone(),
                display_name: view.descriptor.display_name.clone(),
                is_active: now.saturating_sub(modified_secs(&metadata)) < ACTIVE_WINDOW_SECS,
                summary,
            };
            let replace = by_session
                .get(&record.session_id)
                .is_none_or(|previous| record_is_preferred(&record, previous));
            if replace {
                by_session.insert(record.session_id.clone(), record);
            }
        }
        records.extend(by_session.into_values());
    }

    let entry_count_before_retain = cache.entries.len();
    cache
        .entries
        .retain(|key, _| seen_cache_keys.contains(key) || !key_belongs_to_project(key, resolved));
    cache_changed |= cache.entries.len() != entry_count_before_retain;
    if cache.entries.len() > MAX_ACTIVITY_CACHE_ENTRIES {
        warnings.push(format!(
            "Local activity cache exceeded {MAX_ACTIVITY_CACHE_ENTRIES} entries and was reset"
        ));
        cache.entries.clear();
        cache_changed = true;
    }
    cache.schema = ACTIVITY_CACHE_SCHEMA;
    if cache_changed {
        if let Err(error) = write_json_atomic(
            repository.root(),
            &cache_path,
            &cache,
            MAX_ACTIVITY_CACHE_BYTES,
        ) {
            warnings.push(format!(
                "Could not update local activity cache '{}': {error}",
                cache_path.display()
            ));
        }
    }
    Ok(records)
}

fn key_belongs_to_project(key: &str, resolved: &ResolvedActivityProject) -> bool {
    key.split_once('\0')
        .is_some_and(|(project_id, _)| project_id == resolved.project_id.as_str())
}

fn record_is_preferred(candidate: &LocalSessionRecord, current: &LocalSessionRecord) -> bool {
    candidate
        .summary
        .metrics_complete
        .cmp(&current.summary.metrics_complete)
        .then_with(|| {
            candidate
                .summary
                .ended_at
                .unwrap_or(0)
                .cmp(&current.summary.ended_at.unwrap_or(0))
        })
        .is_gt()
}

// ---------------------------------------------------------------------------
// Remote-session metadata
// ---------------------------------------------------------------------------

/// One remote conversation row read from a verified manifest. Reading these
/// is not acknowledgment: no scope base moves and no provider file is
/// touched.
#[derive(Clone, Debug)]
pub struct RemoteSessionRecord {
    pub provider: ProviderKey,
    pub thread_id: CloudThreadId,
    pub source_thread_id: Option<String>,
    pub display_title: Option<String>,
    pub preserved: Option<PreservedProvenance>,
    pub presentation: mallard_domain::SessionPresentation,
}

pub fn remote_sessions_from_snapshot(snapshot: &BundleSnapshot) -> Vec<RemoteSessionRecord> {
    snapshot
        .manifest
        .all_conversations()
        .filter_map(|(scope, thread_id, thread)| match scope {
            SyncScopeKey::Provider(provider) => Some(RemoteSessionRecord {
                provider: provider.clone(),
                thread_id: thread_id.clone(),
                source_thread_id: thread.source_thread_id.clone(),
                display_title: thread.display_title.clone(),
                preserved: thread.preserved.clone(),
                presentation: thread.presentation.clone(),
            }),
            SyncScopeKey::Project => None,
        })
        .collect()
}

/// Fetch the verified current snapshot for one linked storage, for remote
/// Activity rows. Errors are reported as warnings by the caller so a broken
/// storage cannot hide local history.
pub fn fetch_activity_snapshot(
    config: &SyncConfig,
    storage_id: &StorageId,
    bundle_id: &mallard_domain::BundleId,
) -> Result<Option<BundleSnapshot>, String> {
    let storage = config
        .storages
        .iter()
        .find(|storage| &storage.id == storage_id)
        .ok_or_else(|| format!("storage '{}' is not configured", storage_id))?;
    let store = ConfiguredObjectStore::shared(storage)?;
    let engine = crate::bundle_engine::BundleEngine::open(store, storage.id.clone())?
        .with_object_cache(crate::bundle_engine::ObjectCache::global());
    engine.inspect_optional(bundle_id)
}

// ---------------------------------------------------------------------------
// Merged Activity
// ---------------------------------------------------------------------------

fn provider_access(
    registry: &AgentProviderRegistry,
    resolved: &ResolvedActivityProject,
    provider: &ProviderKey,
) -> ProviderAccessState {
    if resolved.homes.contains_key(provider) {
        ProviderAccessState::BoundReady
    } else if resolved.unavailable.contains_key(provider) {
        ProviderAccessState::BoundUnavailable
    } else if registry.contains(provider) {
        ProviderAccessState::ViewOnly
    } else {
        ProviderAccessState::Unsupported
    }
}

fn provider_display_name(registry: &AgentProviderRegistry, provider: &ProviderKey) -> String {
    registry
        .get(provider)
        .map(|adapter| adapter.descriptor().display_name)
        .unwrap_or_else(|| provider.to_string())
}

fn short_id(value: &str) -> &str {
    value.get(..8).unwrap_or(value)
}

#[derive(Default)]
struct ActivityConversationAliases {
    thread_by_session: BTreeMap<(ProviderKey, String), ThreadId>,
    session_by_thread: BTreeMap<(ProviderKey, ThreadId), String>,
    historical_owners: BTreeMap<(ProviderKey, String), BTreeSet<ThreadId>>,
}

fn activity_conversation_aliases(
    resolved: &ResolvedActivityProject,
    bindings: &ConversationBindings,
    materializations: &Materializations,
) -> ActivityConversationAliases {
    let provider_by_profile = resolved
        .binding
        .profile_ids
        .iter()
        .map(|(provider, profile_id)| (profile_id, provider))
        .collect::<BTreeMap<_, _>>();
    let mut aliases = ActivityConversationAliases::default();

    for binding in &bindings.bindings {
        if binding.host_id != resolved.binding.host_id
            || binding.replica_id != resolved.binding.replica_id
            || binding.local_project_id != resolved.project_id
        {
            continue;
        }
        let Some(provider) = provider_by_profile.get(&binding.provider_profile_id) else {
            continue;
        };
        let Some(provider_thread_id) = &binding.provider_thread_id else {
            continue;
        };
        aliases.thread_by_session.insert(
            ((*provider).clone(), provider_thread_id.clone()),
            binding.thread_id.clone(),
        );
        aliases.session_by_thread.insert(
            ((*provider).clone(), binding.thread_id.clone()),
            provider_thread_id.clone(),
        );
    }

    for record in &materializations.records {
        if record.host_id != resolved.binding.host_id
            || record.replica_id != resolved.binding.replica_id
            || record.local_project_id != resolved.project_id
            || record.bundle_id != resolved.bundle_id
        {
            continue;
        }
        for receipt in &record.receipts {
            if receipt.status != ActionStatus::Applied {
                continue;
            }
            let Some(conversation) = &receipt.conversation else {
                continue;
            };
            if conversation.host_id != resolved.binding.host_id
                || conversation.replica_id != resolved.binding.replica_id
                || conversation.materialization_state != ConversationMaterializationState::Resumable
            {
                continue;
            }
            let Some(provider) = provider_by_profile.get(&conversation.provider_profile_id) else {
                continue;
            };
            let Ok((resource_provider, resource_thread_id)) =
                parse_conversation_resource_id(receipt.resource_id.as_str())
            else {
                continue;
            };
            if &resource_provider != *provider || resource_thread_id != conversation.thread_id {
                continue;
            }
            let Some(provider_thread_id) = &conversation.provider_thread_id else {
                continue;
            };
            aliases
                .historical_owners
                .entry(((*provider).clone(), provider_thread_id.clone()))
                .or_default()
                .insert(conversation.thread_id.clone());
        }
    }

    aliases
}

fn displaced_provider_alias_owners(
    bundle_id: &mallard_domain::BundleId,
    provider: &ProviderKey,
    provider_thread_id: &str,
    aliases: &ActivityConversationAliases,
) -> Result<BTreeSet<ThreadId>, String> {
    let current_alias_moved = |thread_id: &ThreadId| {
        aliases
            .session_by_thread
            .get(&(provider.clone(), thread_id.clone()))
            .is_some_and(|current| current != provider_thread_id)
    };

    let mut owners = aliases
        .historical_owners
        .get(&(provider.clone(), provider_thread_id.to_string()))
        .cloned()
        .unwrap_or_default();
    owners.insert(initial_conversation_thread_id(
        bundle_id,
        provider,
        provider_thread_id,
    )?);
    owners.retain(current_alias_moved);
    Ok(owners)
}

/// Resolve one displaced, unbound provider alias into its stable retained
/// branch identity and prove whether its physical bytes still exactly match
/// a copy-on-write witness. Missing, partial, unreadable, or mismatching
/// evidence always fails open as visible local work.
#[allow(clippy::too_many_arguments)]
pub fn retained_provider_alias_identity(
    bundle_id: &mallard_domain::BundleId,
    local_project_id: &LocalProjectId,
    provider: &ProviderKey,
    provider_thread_id: &str,
    physical_path: &Path,
    moved_owners: &BTreeSet<ThreadId>,
    bound_thread_ids: &BTreeSet<ThreadId>,
    config: &SyncConfig,
) -> Result<(ThreadId, bool), String> {
    let mut frontier = moved_owners.clone();
    let mut candidates = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for _ in 0..=bound_thread_ids.len() {
        let mut next = BTreeSet::new();
        for owner in frontier {
            if !visited.insert(owner.clone()) {
                continue;
            }
            let retained =
                retained_conversation_thread_id(bundle_id, provider, &owner, provider_thread_id)?;
            if bound_thread_ids.contains(&retained) {
                next.insert(retained);
            } else {
                candidates.insert(retained);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    let mut witnessed = Vec::new();
    for retained_thread_id in &candidates {
        for checkpoint in config
            .links
            .iter()
            .filter(|link| &link.local_project_id == local_project_id)
            .filter_map(|link| {
                link.conversation_checkpoints
                    .get(&SyncScopeKey::Provider(provider.clone()))
                    .and_then(|threads| threads.get(retained_thread_id))
            })
        {
            if crate::codex_history_graph::retained_root_is_byte_identical(
                physical_path,
                checkpoint,
            )
            .unwrap_or(false)
            {
                return Ok((retained_thread_id.clone(), true));
            }
            witnessed.push(retained_thread_id.clone());
        }
    }
    let retained_thread_id = witnessed
        .into_iter()
        .max()
        .or_else(|| candidates.into_iter().max())
        .ok_or_else(|| "displaced provider alias has no logical owner".to_string())?;
    Ok((retained_thread_id, false))
}

fn activity_thread_for_provider_alias(
    resolved: &ResolvedActivityProject,
    provider: &ProviderKey,
    provider_thread_id: &str,
    physical_path: &Path,
    aliases: &ActivityConversationAliases,
    config: &SyncConfig,
) -> Result<Option<ThreadId>, String> {
    if let Some(thread_id) = aliases
        .thread_by_session
        .get(&(provider.clone(), provider_thread_id.to_string()))
    {
        // A live binding is authoritative even when an older receipt says
        // that another logical thread once occupied the same native alias.
        return Ok(Some(thread_id.clone()));
    }
    let moved_owners = displaced_provider_alias_owners(
        &resolved.bundle_id,
        provider,
        provider_thread_id,
        aliases,
    )?;
    if !moved_owners.is_empty() {
        let (retained_thread_id, unchanged) = retained_provider_alias_identity(
            &resolved.bundle_id,
            &resolved.project_id,
            provider,
            provider_thread_id,
            physical_path,
            &moved_owners,
            &aliases
                .session_by_thread
                .keys()
                .map(|(_, id)| id.clone())
                .collect(),
            config,
        )?;
        return Ok((!unchanged).then_some(retained_thread_id));
    }
    initial_conversation_thread_id(&resolved.bundle_id, provider, provider_thread_id).map(Some)
}

/// Resolve one discovered local provider alias through the same direct-
/// binding and retained-copy witness rules used by Activity. This is the
/// single-session ownership gate used by Session Detail and provider actions;
/// callers must not independently guess an unbound logical identity.
pub fn resolve_local_provider_alias_thread(
    repository: &LocalStateRepository,
    resolved: &ResolvedActivityProject,
    provider: &ProviderKey,
    provider_thread_id: &str,
    physical_path: &Path,
) -> Result<Option<ThreadId>, String> {
    let state = repository.load_project_activity_state_snapshot()?;
    resolve_local_provider_alias_thread_from_state(
        &state,
        resolved,
        provider,
        provider_thread_id,
        physical_path,
    )
}

pub fn resolve_local_provider_alias_thread_from_state(
    state: &LocalProjectActivityStateSnapshot,
    resolved: &ResolvedActivityProject,
    provider: &ProviderKey,
    provider_thread_id: &str,
    physical_path: &Path,
) -> Result<Option<ThreadId>, String> {
    if state.bindings.active_for(&resolved.project_id) != Some(&resolved.binding)
        || resolved.homes.iter().any(|(provider, home)| {
            !state.bindings.profiles.iter().any(|profile| {
                profile.profile_id == home.profile_id
                    && &profile.provider == provider
                    && Path::new(&profile.canonical_path) == home.home
            })
        })
    {
        return Err("project binding changed; refresh before continuing".to_string());
    }
    let aliases = activity_conversation_aliases(
        resolved,
        &state.conversation_bindings,
        &state.materializations,
    );
    activity_thread_for_provider_alias(
        resolved,
        provider,
        provider_thread_id,
        physical_path,
        &aliases,
        &state.config,
    )
}

/// Merge local bound-provider sessions with all remote provider slices by
/// provider and logical thread identity, then sort deterministically by
/// activity time, provider key, and resource ID.
pub fn merge_activity_sessions(
    registry: &AgentProviderRegistry,
    repository: &LocalStateRepository,
    resolved: &ResolvedActivityProject,
    local: Vec<LocalSessionRecord>,
    remote: Vec<RemoteSessionRecord>,
    warnings: &mut Vec<String>,
) -> Result<Vec<ProjectSessionSummary>, String> {
    let state = repository.load_project_activity_state_snapshot()?;
    merge_activity_sessions_with_state(
        registry,
        resolved,
        local,
        remote,
        warnings,
        &state.conversation_bindings,
        &state.materializations,
        &state.config,
    )
}

#[allow(clippy::too_many_arguments)]
fn merge_activity_sessions_with_state(
    registry: &AgentProviderRegistry,
    resolved: &ResolvedActivityProject,
    local: Vec<LocalSessionRecord>,
    remote: Vec<RemoteSessionRecord>,
    warnings: &mut Vec<String>,
    bindings: &ConversationBindings,
    materializations: &Materializations,
    config: &SyncConfig,
) -> Result<Vec<ProjectSessionSummary>, String> {
    let preserved_threads = bindings
        .bindings
        .iter()
        .filter(|binding| {
            binding.host_id == resolved.binding.host_id
                && binding.replica_id == resolved.binding.replica_id
                && binding.local_project_id == resolved.project_id
                && resolved
                    .binding
                    .profile_ids
                    .values()
                    .any(|profile_id| profile_id == &binding.provider_profile_id)
        })
        .filter_map(|binding| {
            binding
                .preserved
                .clone()
                .map(|preserved| (binding.thread_id.clone(), preserved))
        })
        .collect::<BTreeMap<ThreadId, PreservedProvenance>>();
    // The only local↔cloud join in Activity: the scoped binding rows'
    // mappings. Keyed by profile so a bundle-global cloud id resolves within
    // its own provider's rows.
    let cloud_to_local = bindings
        .bindings
        .iter()
        .filter(|binding| {
            binding.host_id == resolved.binding.host_id
                && binding.replica_id == resolved.binding.replica_id
                && binding.local_project_id == resolved.project_id
                && resolved
                    .binding
                    .profile_ids
                    .values()
                    .any(|profile_id| profile_id == &binding.provider_profile_id)
        })
        .filter_map(|binding| {
            binding.cloud_thread_id.clone().map(|cloud_thread_id| {
                (
                    (binding.provider_profile_id.clone(), cloud_thread_id),
                    binding.thread_id.clone(),
                )
            })
        })
        .collect::<BTreeMap<(LocalProviderProfileId, CloudThreadId), ThreadId>>();
    let aliases = activity_conversation_aliases(resolved, bindings, materializations);

    let mut rows: BTreeMap<String, ProjectSessionSummary> = BTreeMap::new();
    for record in local {
        let thread_id = match activity_thread_for_provider_alias(
            resolved,
            &record.provider,
            &record.session_id,
            &record.physical_path,
            &aliases,
            config,
        ) {
            Ok(thread_id) => thread_id,
            Err(error) => {
                warnings.push(format!(
                    "session '{}' has no portable identity: {}",
                    record.session_id, error
                ));
                None
            }
        };
        let Some(thread_id) = thread_id else {
            continue;
        };
        let resource_id = conversation_resource_id(&record.provider, &thread_id)?;
        let access = provider_access(registry, resolved, &record.provider);
        let title = (record.display_name != record.session_id)
            .then(|| record.display_name.clone())
            .filter(|title| !title.trim().is_empty())
            .or_else(|| {
                record
                    .summary
                    .title
                    .clone()
                    .filter(|title| !title.trim().is_empty())
            })
            .unwrap_or_else(|| {
                format!(
                    "{} session {}",
                    provider_display_name(registry, &record.provider),
                    short_id(&record.session_id)
                )
            });
        let actions = registry
            .get(&record.provider)
            .map(|adapter| {
                adapter.available_actions(&SessionActionContext {
                    provider_session_id: &record.session_id,
                    is_local: access == ProviderAccessState::BoundReady,
                })
            })
            .unwrap_or_default();
        let started_at = record.summary.started_at.unwrap_or(0);
        let ended_at = record
            .summary
            .ended_at
            .unwrap_or(started_at)
            .max(started_at);
        rows.insert(
            resource_id.to_string(),
            ProjectSessionSummary {
                resource_id: resource_id.to_string(),
                provider: record.provider.clone(),
                provider_display_name: provider_display_name(registry, &record.provider),
                provider_session_id: Some(record.session_id),
                logical_thread_id: Some(thread_id.to_string()),
                source: SessionSourceState::LocalOnly,
                access,
                title,
                preserved: preserved_threads.get(&thread_id).cloned(),
                summary: record.summary.summary.clone(),
                started_at,
                ended_at,
                last_prompt_at: record.summary.last_prompt_at,
                transcript_path: Some(record.physical_path.to_string_lossy().into_owned()),
                working_directory: Some(resolved.project_root.to_string_lossy().into_owned()),
                sync_blocked_reason: None,
                recorded_sha: record.summary.recorded_sha.clone(),
                is_active: record.is_active,
                user_round_count: record.summary.user_rounds,
                agent_message_count: record.summary.agent_messages,
                tool_call_count: record.summary.tool_calls,
                total_input_tokens: record.summary.total_input_tokens,
                total_output_tokens: record.summary.total_output_tokens,
                total_tokens: record.summary.total_tokens,
                metrics_complete: record.summary.metrics_complete,
                actions,
            },
        );
    }

    for record in remote {
        // The shared row-key rule: a cloud row claimed by a scoped binding
        // keys by the local id (joining the local session row when one was
        // discovered); an unclaimed row is "only in this storage" and keys
        // by its own cloud id. The namespaces are prefix-disjoint, so the
        // two key forms can never collide.
        let claimed_local =
            resolved
                .binding
                .profile_ids
                .get(&record.provider)
                .and_then(|profile_id| {
                    cloud_to_local.get(&(profile_id.clone(), record.thread_id.clone()))
                });
        let key = match claimed_local {
            Some(thread_id) => conversation_resource_id(&record.provider, thread_id)?.to_string(),
            None => {
                cloud_conversation_resource_id(&record.provider, &record.thread_id)?.to_string()
            }
        };
        if let Some(existing) = rows.get_mut(&key) {
            existing.source = SessionSourceState::LocalAndRemote;
            if existing.title.is_empty() {
                if let Some(title) = &record.display_title {
                    existing.title = title.clone();
                }
            }
            if existing.preserved.is_none() {
                existing.preserved = record.preserved.clone();
            }
            continue;
        }
        let access = provider_access(registry, resolved, &record.provider);
        let title = record
            .display_title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| {
                format!(
                    "{} session {}",
                    provider_display_name(registry, &record.provider),
                    short_id(
                        record
                            .source_thread_id
                            .as_deref()
                            .unwrap_or(record.thread_id.as_str())
                    )
                )
            });
        let started_at = record.presentation.started_at.unwrap_or(0);
        let ended_at = record
            .presentation
            .ended_at
            .unwrap_or(started_at)
            .max(started_at);
        rows.insert(
            key.clone(),
            ProjectSessionSummary {
                resource_id: key,
                provider: record.provider.clone(),
                provider_display_name: provider_display_name(registry, &record.provider),
                provider_session_id: record.source_thread_id.clone(),
                logical_thread_id: Some(record.thread_id.to_string()),
                source: SessionSourceState::RemoteOnly,
                access,
                title,
                preserved: record.preserved.clone(),
                summary: None,
                started_at,
                ended_at,
                last_prompt_at: record.presentation.last_prompt_at,
                transcript_path: None,
                working_directory: None,
                sync_blocked_reason: None,
                recorded_sha: record.presentation.recorded_sha.clone(),
                is_active: false,
                user_round_count: record.presentation.metrics.user_rounds,
                agent_message_count: record.presentation.metrics.agent_messages,
                tool_call_count: record.presentation.metrics.tool_calls,
                total_input_tokens: record.presentation.metrics.total_input_tokens,
                total_output_tokens: record.presentation.metrics.total_output_tokens,
                total_tokens: record.presentation.metrics.total_tokens,
                metrics_complete: record.presentation.metrics.complete,
                actions: Vec::new(),
            },
        );
    }

    let mut sessions: Vec<ProjectSessionSummary> = rows.into_values().collect();
    sessions.sort_by(|left, right| {
        right
            .ended_at
            .cmp(&left.ended_at)
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.resource_id.cmp(&right.resource_id))
    });
    Ok(sessions)
}

// ---------------------------------------------------------------------------
// The complete Activity view
// ---------------------------------------------------------------------------

/// What lies before the window, weighed over every commit the branch offered
/// and every session the project has.
///
/// The cursor is the newest commit or session below the window, so the control
/// exists exactly when something is there and one press lands on it.
///
/// A commit before the window still draws when a visible session maps to it,
/// so those are excluded here: counting them would promise rows that are
/// already on the page. Sessions need no such guard, because the session list
/// is strictly windowed.
fn older_activity(
    branch_commits: &[GitCommitSummary],
    session_ends: &[u64],
    window_start: u64,
) -> Option<OlderActivity> {
    let mut newest_at: Option<u64> = None;
    let mut commits = 0usize;
    for commit in branch_commits {
        if commit.committed_at < window_start && commit.session_refs.is_empty() {
            commits += 1;
            newest_at = newest_at.max(Some(commit.committed_at));
        }
    }
    let mut sessions = 0usize;
    for ended_at in session_ends {
        if *ended_at < window_start {
            sessions += 1;
            newest_at = newest_at.max(Some(*ended_at));
        }
    }
    newest_at.map(|newest_at| OlderActivity {
        newest_at,
        commits,
        sessions,
    })
}

/// Project the Activity response from one already-resolved project, one local
/// discovery witness, and one verified remote snapshot projection.
#[allow(clippy::too_many_arguments)]
pub fn build_project_activity(
    registry: &AgentProviderRegistry,
    resolved: &ResolvedActivityProject,
    bindings: &ConversationBindings,
    materializations: &Materializations,
    config: &SyncConfig,
    local: Vec<LocalSessionRecord>,
    remote: Vec<RemoteSessionRecord>,
    branch: Option<&str>,
    before_time: Option<u64>,
    window_days: Option<u64>,
    review: bool,
    mut warnings: Vec<String>,
) -> Result<ProjectActivity, String> {
    let remote_providers = remote
        .iter()
        .map(|record| record.provider.clone())
        .collect::<BTreeSet<_>>();
    let all_sessions = merge_activity_sessions_with_state(
        registry,
        resolved,
        local,
        remote,
        &mut warnings,
        bindings,
        materializations,
        config,
    )?;

    let window_end = before_time.unwrap_or_else(|| now_secs().saturating_add(1));
    let window_days = window_days
        .unwrap_or(DEFAULT_HISTORY_WINDOW_DAYS)
        .clamp(1, 90);
    let window_start = window_end.saturating_sub(window_days.saturating_mul(DAY_SECS));
    let sessions = all_sessions
        .iter()
        .filter(|session| {
            review || (session.ended_at >= window_start && session.ended_at < window_end)
        })
        .cloned()
        .collect::<Vec<_>>();

    let git = discover_git_branches(&resolved.project_root, branch)?;
    let mut unmapped = Vec::new();
    // Every first-parent commit the branch offered, windowed or not. Carried
    // out of the block below so `older_activity` can weigh what lies before
    // the window against the sessions.
    let mut branch_commits: Vec<GitCommitSummary> = Vec::new();
    let git_page = if let Some(discovery) = git {
        let github_repository_url = discover_github_repository_url(&resolved.project_root);
        let all_commits = if discovery.selected_has_ref {
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
        let mapping = map_sessions_to_commits(&sessions, &mut all_commits);
        unmapped = mapping.unmapped;
        let visible_commits = all_commits
            .iter()
            .filter(|commit| {
                (commit.committed_at >= window_start && commit.committed_at < window_end)
                    || !commit.session_refs.is_empty()
            })
            .cloned()
            .collect::<Vec<_>>();
        // After the mapping, so the refs a commit picked up are already on it.
        branch_commits = all_commits;
        Some(GitHistoryPage {
            selected_branch: discovery.selected_branch,
            branches: discovery.branches,
            commits: visible_commits,
            github_repository_url,
            unique_session_count: mapping.unique_session_count,
        })
    } else {
        None
    };

    // Per-provider access rows: every bound provider plus every remote-only
    // provider, so view-only and unsupported states render distinctly.
    let mut provider_keys = resolved
        .binding
        .profile_ids
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    provider_keys.extend(remote_providers);
    let providers = provider_keys
        .into_iter()
        .map(|provider| ProviderActivityState {
            display_name: provider_display_name(registry, &provider),
            access: provider_access(registry, resolved, &provider),
            detail: resolved.unavailable.get(&provider).cloned(),
            provider,
        })
        .collect();

    let older = older_activity(
        &branch_commits,
        &all_sessions
            .iter()
            .map(|session| session.ended_at)
            .collect::<Vec<_>>(),
        window_start,
    );
    Ok(ProjectActivity {
        project_id: resolved.project_id.to_string(),
        sessions,
        git: git_page,
        unmapped,
        providers,
        warnings,
        window_start,
        window_end,
        older,
        storage_sync: project_storage_sync(config, &resolved.project_id, materializations),
    })
}

fn project_storage_sync(
    config: &SyncConfig,
    local_project_id: &LocalProjectId,
    materializations: &Materializations,
) -> Vec<StorageSyncSummary> {
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
        let last_pull_at = link
            .scope_bases
            .values()
            .filter_map(|base| base.last_pull_at)
            .max()
            .or(historical_pull);
        let last_push_at = link
            .scope_bases
            .values()
            .filter_map(|base| base.last_push_at)
            .max();
        result.push(StorageSyncSummary {
            storage_id: link.storage_id.to_string(),
            storage_name,
            last_pull_at,
            last_push_at,
        });
    }
    result.sort_by(|left, right| left.storage_name.cmp(&right.storage_name));
    result
}

pub fn list_project_repository_kinds(
    repository: &LocalStateRepository,
) -> Result<BTreeMap<String, bool>, String> {
    let state = repository.load_project_state_snapshot()?;
    Ok(state
        .config
        .projects
        .iter()
        .filter_map(|project| {
            let root = state
                .bindings
                .bindings
                .iter()
                .find(|binding| {
                    binding.local_project_id == project.local_project_id
                        && binding.state == BindingState::Active
                })
                .and_then(|binding| fs::canonicalize(&binding.project_root).ok())?;
            Some((
                project.local_project_id.to_string(),
                is_git_work_tree(&root),
            ))
        })
        .collect())
}

/// Whether Git counts `root` as inside a work tree — the one rule that
/// decides whether a project's ordinary files travel through Git.
pub fn is_git_work_tree(root: &Path) -> bool {
    git_output(root, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|output| {
        output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
    })
}

/// The Git metadata a checkout or a commit rewrites: the worktree's own git
/// dir, which holds `HEAD`, and the common dir every worktree shares, which
/// holds `refs/heads` and `packed-refs`. Both are canonical. In an ordinary
/// clone they are the same directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectGitDirs {
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
}

/// The Git dirs of a registered project's active checkout, or `None` when the
/// project has no active binding or is not a Git work tree.
pub fn project_git_dirs(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<Option<ProjectGitDirs>, String> {
    let state = repository.load_project_state_snapshot()?;
    let root = state
        .bindings
        .bindings
        .iter()
        .find(|binding| {
            &binding.local_project_id == local_project_id && binding.state == BindingState::Active
        })
        .and_then(|binding| fs::canonicalize(&binding.project_root).ok());
    Ok(root.and_then(|root| git_dirs(&root)))
}

pub fn git_dirs(root: &Path) -> Option<ProjectGitDirs> {
    let output = git_output(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
    )
    .ok()
    .filter(|output| output.status.success())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let git_dir = fs::canonicalize(lines.next()?).ok()?;
    let common_dir = fs::canonicalize(lines.next()?).ok()?;
    Some(ProjectGitDirs {
        git_dir,
        common_dir,
    })
}

// ---------------------------------------------------------------------------
// Session details and actions
// ---------------------------------------------------------------------------

/// Execute one safe provider session action. The backend re-resolves the
/// registered adapter and validates project ownership before anything runs;
/// the opaque action id never carries executable text.
pub fn execute_project_session_action(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    local_project_id: &LocalProjectId,
    resource_id: &str,
    action_id: &str,
    before_launch: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let state = repository.load_project_activity_state_snapshot()?;
    let resolved =
        resolve_activity_project_from_state(&state.config, &state.bindings, local_project_id)?;
    let (provider, _, session) = resolve_action_session(registry, &state, &resolved, resource_id)?;
    let adapter = registry.require(&provider)?;
    let home = &resolved.homes[&provider];
    let settings = crate::app_settings::load_app_settings(repository)?;
    let mut request = SessionActionRequest {
        action_id,
        provider_home: &home.home,
        project_root: &resolved.project_root,
        provider_session_id: &session.session_id,
        cli_path: settings.cli_path(&provider),
        before_launch,
    };
    adapter.execute_action(&mut request).map(|_| ())
}

fn resolve_action_session(
    registry: &AgentProviderRegistry,
    state: &LocalProjectActivityStateSnapshot,
    resolved: &ResolvedActivityProject,
    resource_id: &str,
) -> Result<(ProviderKey, ThreadId, crate::providers::DiscoveredSession), String> {
    let (provider, thread_id) = parse_conversation_resource_id(resource_id)?;
    let session_for_thread = state
        .conversation_bindings
        .bindings
        .iter()
        .find(|binding| {
            binding.local_project_id == resolved.project_id
                && binding.thread_id == thread_id
                && resolved
                    .homes
                    .get(&provider)
                    .is_some_and(|home| home.profile_id == binding.provider_profile_id)
        })
        .and_then(|binding| binding.provider_thread_id.clone());
    let adapter = registry.require(&provider)?;
    let home = resolved.homes.get(&provider).ok_or_else(|| {
        format!(
            "provider '{}' is not bound with a readable profile on this machine",
            provider
        )
    })?;
    let context = SessionDiscoveryContext {
        provider_home: &home.home,
        project_root: &resolved.project_root,
    };
    // A bound thread names its provider session outright. An unbound one is
    // known only through the alias rules, applied to each candidate the
    // adapter offers; neither test opens a transcript.
    let session = adapter
        .locate_session(&context, &mut |session_id, physical_path| {
            Ok(match session_for_thread.as_deref() {
                Some(expected) => expected == session_id,
                None => {
                    resolve_local_provider_alias_thread_from_state(
                        state,
                        resolved,
                        &provider,
                        session_id,
                        physical_path,
                    )?
                    .as_ref()
                        == Some(&thread_id)
                }
            })
        })?
        .ok_or_else(|| "session does not belong to the selected project".to_string())?;
    Ok((provider, thread_id, session))
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LocalSessionDeleteResult {
    pub provider_error: Option<String>,
    pub metadata_error: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectSessionsDeleteResult {
    pub sessions_deleted: u64,
    pub sessions_failed: u64,
    pub provider_errors: Vec<String>,
    pub metadata_reset: Option<crate::persistence::ProjectSyncMetadataResetResult>,
    pub metadata_error: Option<String>,
}

/// Delete discovered sessions in the project's bound profiles, then reset all
/// project sync metadata. Each provider and the metadata reset are best effort.
pub fn delete_local_project_sessions(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    local_project_id: &LocalProjectId,
    expected_project_revision: u64,
) -> Result<ProjectSessionsDeleteResult, String> {
    let state = repository.load_project_state_snapshot()?;
    let project = state
        .config
        .project(local_project_id)
        .ok_or("Unknown local project.")?;
    if project.revision != expected_project_revision {
        return Err("The project changed. Refresh before deleting its sessions.".into());
    }
    let mut result = ProjectSessionsDeleteResult {
        sessions_deleted: 0,
        sessions_failed: 0,
        provider_errors: Vec::new(),
        metadata_reset: None,
        metadata_error: None,
    };
    match resolve_activity_project_from_state(&state.config, &state.bindings, local_project_id) {
        Ok(resolved) => {
            for (provider, error) in &resolved.unavailable {
                if matches!(provider.as_str(), "codex" | "claude") {
                    result.provider_errors.push(format!("{provider}: {error}"));
                }
            }
            let settings = crate::app_settings::load_app_settings(repository)?;
            for (provider, home) in &resolved.homes {
                if !matches!(provider.as_str(), "codex" | "claude") {
                    continue;
                }
                let discovery = registry.require(provider).and_then(|adapter| {
                    adapter
                        .discover(&SessionDiscoveryContext {
                            provider_home: &home.home,
                            project_root: &resolved.project_root,
                        })
                        .map(|discovery| (adapter, discovery))
                });
                match discovery {
                    Ok((adapter, discovery)) => {
                        result.provider_errors.extend(discovery.warnings);
                        for session in discovery.sessions {
                            match adapter.delete_session(
                                &home.home,
                                &session,
                                settings.cli_path(provider),
                            ) {
                                Ok(()) => result.sessions_deleted += 1,
                                Err(error) => {
                                    result.sessions_failed += 1;
                                    result.provider_errors.push(format!(
                                        "{provider} session {}: {error}",
                                        session.session_id
                                    ));
                                }
                            }
                        }
                    }
                    Err(error) => result.provider_errors.push(format!("{provider}: {error}")),
                }
            }
        }
        Err(error) => result.provider_errors.push(error),
    }
    // Do not return early on provider failures: stale and restored bindings
    // for this project must also be removed when files are unavailable.
    match repository.reset_project_sync_metadata(local_project_id, expected_project_revision) {
        Ok(reset) => result.metadata_reset = Some(reset),
        Err(error) => result.metadata_error = Some(error),
    }
    Ok(result)
}

/// One provider attempt followed by independent, targeted Mallard cleanup.
pub fn delete_local_project_session(
    repository: &LocalStateRepository,
    registry: &AgentProviderRegistry,
    local_project_id: &LocalProjectId,
    resource_id: &str,
) -> Result<LocalSessionDeleteResult, String> {
    let state = repository.load_project_activity_state_snapshot()?;
    let resolved =
        resolve_activity_project_from_state(&state.config, &state.bindings, local_project_id)?;
    let (provider, thread_id) = parse_conversation_resource_id(resource_id)?;
    let adapter = registry.require(&provider)?;
    let home = resolved
        .homes
        .get(&provider)
        .ok_or("The provider profile is unavailable.")?;
    let bound = state.conversation_bindings.bindings.iter().any(|binding| {
        binding.local_project_id == *local_project_id
            && binding.provider_profile_id == home.profile_id
            && binding.thread_id == thread_id
    });
    let settings = crate::app_settings::load_app_settings(repository)?;
    let provider_error = match resolve_action_session(registry, &state, &resolved, resource_id) {
        Ok((_, _, session)) => adapter
            .delete_session(&home.home, &session, settings.cli_path(&provider))
            .err(),
        // A missing or unreadable provider file must not prevent known binding cleanup.
        Err(error) if bound => Some(error),
        Err(error) => return Err(error),
    };
    let metadata_error = repository
        .forget_local_session(local_project_id, &home.profile_id, &provider, &thread_id)
        .err();
    Ok(LocalSessionDeleteResult {
        provider_error,
        metadata_error,
    })
}

// ---------------------------------------------------------------------------
// Git discovery and mapping over normalized evidence
// ---------------------------------------------------------------------------

struct MappingResult {
    unmapped: Vec<UnmappedSessionReference>,
    unique_session_count: usize,
}

struct GitBranchDiscovery {
    selected_branch: String,
    selected_has_ref: bool,
    branches: Vec<GitBranchSummary>,
}

fn discover_git_branches(
    project_root: &Path,
    requested_branch: Option<&str>,
) -> Result<Option<GitBranchDiscovery>, String> {
    let inside = git_output(project_root, &["rev-parse", "--is-inside-work-tree"])
        .ok()
        .filter(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
        });
    if inside.is_none() {
        return Ok(None);
    }
    let current = git_output(project_root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|name| !name.is_empty());
    let listed = git_output(
        project_root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )?;
    if !listed.status.success() {
        return Err(git_error("enumerate local branches", &listed));
    }
    let available_names = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    let mut names = available_names.clone();
    if let Some(current) = &current {
        names.insert(current.clone());
    }
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
            name,
        })
        .collect::<Vec<_>>();

    // An unborn current branch has a symbolic HEAD but no refs/heads entry
    // until its first commit.
    let selected_has_ref = available_names.contains(&selected);
    Ok(Some(GitBranchDiscovery {
        selected_branch: selected,
        selected_has_ref,
        branches,
    }))
}

fn load_first_parent_commits(
    project_root: &Path,
    branch: &str,
    limit: usize,
) -> Result<Vec<GitCommitSummary>, String> {
    let branch_ref = if branch == "HEAD" {
        branch.to_string()
    } else {
        format!("refs/heads/{branch}")
    };
    read_first_parent_commits(project_root, &branch_ref, limit, "--format=%H%x1f%ct%x1f%s")
}

fn read_first_parent_commits(
    project_root: &Path,
    reference: &str,
    limit: usize,
    format: &str,
) -> Result<Vec<GitCommitSummary>, String> {
    let count = limit.min(MAX_GIT_COMMITS + 1).to_string();
    let output = git_output(
        project_root,
        &[
            "log",
            "--first-parent",
            "--date-order",
            format,
            "-n",
            &count,
            reference,
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
        if !is_valid_git_object_id(sha) {
            continue;
        }
        commits.push(GitCommitSummary {
            sha: sha.to_ascii_lowercase(),
            short_sha: sha.chars().take(12).collect(),
            committed_at: time,
            subject,
            session_refs: Vec::new(),
        });
    }
    Ok(commits)
}

/// Capture the checked-out first-parent rail, including an unborn or detached
/// HEAD. Read by the pinned object id so a concurrent commit cannot mix rails.
pub fn capture_git_history(
    project_root: &Path,
) -> Result<mallard_domain::GitHistorySnapshot, String> {
    let before = git_history_tip(project_root)?;
    let loaded = match &before.1 {
        Some(sha) => read_first_parent_commits(
            project_root,
            sha,
            MAX_GIT_COMMITS + 1,
            "--format=%H%x1f%ct%x1f%<(512,trunc)%s",
        )?,
        None => Vec::new(),
    };
    if before != git_history_tip(project_root)? {
        return Err("Git history changed during capture. Review Push again.".into());
    }
    let shallow = git_output(project_root, &["rev-parse", "--is-shallow-repository"])?;
    if !shallow.status.success() {
        return Err(git_error("check history completeness", &shallow));
    }
    let truncated =
        loaded.len() > MAX_GIT_COMMITS || String::from_utf8_lossy(&shallow.stdout).trim() == "true";
    let commits = loaded
        .into_iter()
        .take(MAX_GIT_COMMITS)
        .map(|commit| {
            let mut subject: String = commit
                .subject
                .trim_end()
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            let mut end = subject.len().min(mallard_domain::MAX_GIT_SUBJECT_BYTES);
            while !subject.is_char_boundary(end) {
                end -= 1;
            }
            subject.truncate(end);
            mallard_domain::GitHistoryCommit {
                sha: commit.sha,
                committed_at: commit.committed_at,
                subject,
                session_refs: Vec::new(),
            }
        })
        .collect();
    let history = mallard_domain::GitHistorySnapshot {
        branch: before.0,
        head_sha: before.1,
        commits,
        truncated,
        uncommitted_session_refs: Vec::new(),
        outside_history_session_refs: Vec::new(),
    };
    history.validate(&BTreeSet::new())?;
    Ok(history)
}

fn git_history_tip(project_root: &Path) -> Result<(String, Option<String>), String> {
    let inside = git_output(project_root, &["rev-parse", "--is-inside-work-tree"])?;
    if !inside.status.success() || String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return Err("The Git project is unavailable. Restore its repository before Push.".into());
    }
    let symbolic = git_output(project_root, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    let branch = if symbolic.status.success() {
        String::from_utf8_lossy(&symbolic.stdout).trim().to_string()
    } else {
        "HEAD".to_string()
    };
    let head = git_output(project_root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let head_sha = if head.status.success() {
        let sha = String::from_utf8_lossy(&head.stdout).trim().to_string();
        if !is_valid_git_object_id(&sha) {
            return Err("Git HEAD is invalid".into());
        }
        Some(sha.to_ascii_lowercase())
    } else {
        // A missing symbolic branch is unborn. Other resolution errors fail.
        let reference = format!("refs/heads/{branch}");
        let exists = git_output(
            project_root,
            &["show-ref", "--verify", "--quiet", &reference],
        )?;
        if branch == "HEAD" || exists.status.code() != Some(1) {
            return Err(git_error("read Git HEAD", &head));
        }
        None
    };
    Ok((branch, head_sha))
}

/// The review token covers only portable metadata, without a local path.
pub fn git_history_review_token(
    history: &mallard_domain::GitHistorySnapshot,
) -> Result<String, String> {
    let bytes =
        serde_json::to_vec(history).map_err(|error| format!("serialize Git history: {error}"))?;
    Ok(mallard_domain::sha256_hex(&bytes))
}

pub fn verify_git_history_review(project_root: &Path, token: &str) -> Result<(), String> {
    if git_history_review_token(&capture_git_history(project_root)?)? != token {
        return Err("Git history changed after review. Review Push again.".into());
    }
    Ok(())
}

/// Map every project session onto exactly one commit of the selected
/// first-parent rail: the closest commit at or after its last human input, or
/// after its end when the transcript recorded no input. A session with no such
/// commit is reported unmapped: it is still uncommitted on this branch.
/// Provider-recorded branches and SHAs are provenance only and never control
/// placement.
fn map_sessions_to_commits(
    sessions: &[ProjectSessionSummary],
    commits: &mut [GitCommitSummary],
) -> MappingResult {
    let mut mapped_sessions = BTreeSet::new();
    let mut unmapped = Vec::new();
    let times: Vec<_> = commits.iter().map(|commit| commit.committed_at).collect();
    for session in sessions {
        let placed_at = session.last_prompt_at.unwrap_or(session.ended_at);
        // Oldest first, so two commits in the same second resolve to the
        // earlier one.
        let closest = mallard_domain::closest_commit_index(&times, placed_at);
        let Some(index) = closest else {
            let reason = if commits.is_empty() {
                "The selected branch has no first-parent commits"
            } else {
                "No first-parent commit on this branch follows the session's last prompt"
            };
            unmapped.push(UnmappedSessionReference {
                resource_id: session.resource_id.clone(),
                reason: reason.to_string(),
            });
            continue;
        };
        mapped_sessions.insert(session.resource_id.clone());
        commits[index].session_refs.push(CommitSessionReference {
            resource_id: session.resource_id.clone(),
        });
    }
    let session_order = sessions
        .iter()
        .map(|session| (session.resource_id.as_str(), session))
        .collect::<BTreeMap<_, _>>();
    for commit in commits.iter_mut() {
        commit.session_refs.sort_by(|left, right| {
            match (
                session_order.get(left.resource_id.as_str()),
                session_order.get(right.resource_id.as_str()),
            ) {
                (Some(left_session), Some(right_session)) => right_session
                    .ended_at
                    .cmp(&left_session.ended_at)
                    .then_with(|| left_session.started_at.cmp(&right_session.started_at))
                    .then_with(|| left.resource_id.cmp(&right.resource_id)),
                _ => left.resource_id.cmp(&right.resource_id),
            }
        });
    }
    debug_assert_eq!(
        mapped_sessions.len().saturating_add(unmapped.len()),
        sessions.len(),
        "every project session must be mapped or explicitly unmapped, once",
    );
    MappingResult {
        unmapped,
        unique_session_count: mapped_sessions.len(),
    }
}

/// The project's canonical GitHub URL, taken from `origin` when it exists and
/// any other GitHub remote otherwise. Session detail reads this too, so a
/// commit marker can offer the same link the activity list does.
pub fn discover_github_repository_url(project_root: &Path) -> Option<String> {
    let output = git_output(
        project_root,
        &["config", "--local", "--get-regexp", r"^remote\..*\.url$"],
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

pub fn normalize_github_repository_url(remote_url: &str) -> Option<String> {
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

fn is_valid_git_object_id(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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

#[cfg(test)]
mod activity_only_tests {
    use super::*;
    #[test]
    fn captured_git_history_handles_unborn_detached_and_changed_heads() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_DATE", "2026-07-22T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2026-07-22T00:00:00Z")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init", "-b", "main"]);
        let unborn = capture_git_history(root).unwrap();
        assert_eq!(unborn.branch, "main");
        assert!(unborn.head_sha.is_none());
        assert!(unborn.commits.is_empty());
        run(&["config", "user.name", "Mallard Test"]);
        run(&["config", "user.email", "mallard@example.test"]);
        run(&["commit", "--allow-empty", "-m", "Initial"]);
        let initial = capture_git_history(root).unwrap();
        let token = git_history_review_token(&initial).unwrap();
        verify_git_history_review(root, &token).unwrap();
        run(&["switch", "-c", "feature"]);
        run(&["commit", "--allow-empty", "-m", "Side branch"]);
        run(&["switch", "main"]);
        run(&["merge", "--no-ff", "feature", "-m", "Merge feature"]);
        let merged = capture_git_history(root).unwrap();
        assert_eq!(
            merged
                .commits
                .iter()
                .map(|commit| commit.subject.as_str())
                .collect::<Vec<_>>(),
            ["Merge feature", "Initial"]
        );
        assert!(verify_git_history_review(root, &token).is_err());
        run(&["checkout", "--detach", initial.head_sha.as_ref().unwrap()]);
        let detached = capture_git_history(root).unwrap();
        assert_eq!(detached.branch, "HEAD");
        assert_eq!(detached.head_sha, initial.head_sha);
        fs::remove_dir_all(root.join(".git")).unwrap();
        assert!(capture_git_history(root).is_err());
    }

    #[test]
    fn captured_git_history_bounds_subjects_and_marks_shallow_checkouts() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        let run = |root: &Path, args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&source, &["init", "-b", "abcdef"]);
        run(&source, &["config", "user.name", "Mallard Test"]);
        run(&source, &["config", "user.email", "mallard@example.test"]);
        run(&source, &["commit", "--allow-empty", "-m", "Initial"]);
        let subject = "é🐥".repeat(600);
        run(&source, &["commit", "--allow-empty", "-m", &subject]);
        let complete = capture_git_history(&source).unwrap();
        assert_eq!(complete.branch, "abcdef");
        assert!(!complete.truncated);
        assert_eq!(complete.commits.len(), 2);
        assert!(!complete.commits[0].subject.is_empty());
        assert!(complete.commits[0].subject.len() <= mallard_domain::MAX_GIT_SUBJECT_BYTES);
        let shallow = temp.path().join("shallow");
        run(
            temp.path(),
            &[
                "clone",
                "--depth",
                "1",
                &format!("file://{}", source.display()),
                shallow.to_str().unwrap(),
            ],
        );
        let captured = capture_git_history(&shallow).unwrap();
        assert!(captured.truncated);
        assert_eq!(captured.commits.len(), 1);
        assert_eq!(captured.head_sha, complete.head_sha);
    }

    use crate::providers::{
        MaterializationTargetRequest, PlannedProviderTarget, ProviderDescriptor,
        SessionActionOutcome, SessionAnalysis, SessionEventDetail, SessionFileMetadata,
        SessionTimelineContext, SessionTimelineIndex, SessionTimelinePageSlice,
        SessionTimelineQuery,
    };
    use mallard_domain::{
        ActionId, ApplyReceipt, BundleId, BundleRecipe, ConversationApplyReceipt,
        ConversationMaterializationState, ConversationOperationId, HostId,
        LocalConversationBinding, LocalProjectRegistration, LocalProviderProfileId,
        MaterializationId, MaterializationRecord, PlanId, ProjectBinding, ProviderProfile,
        ReplicaId, RestoreActionType,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn commit_at(committed_at: u64, session_refs: &[&str]) -> GitCommitSummary {
        GitCommitSummary {
            sha: format!("{committed_at:040}"),
            short_sha: format!("{committed_at:07}"),
            committed_at,
            subject: format!("commit at {committed_at}"),
            session_refs: session_refs
                .iter()
                .map(|resource_id| CommitSessionReference {
                    resource_id: (*resource_id).to_string(),
                })
                .collect(),
        }
    }

    fn session_at(
        provider: &str,
        session_id: &str,
        started_at: u64,
        ended_at: u64,
        recorded_sha: Option<String>,
    ) -> ProjectSessionSummary {
        ProjectSessionSummary {
            resource_id: format!("{provider}:conversation:{session_id}"),
            provider: ProviderKey::parse(provider).unwrap(),
            provider_display_name: provider.to_string(),
            provider_session_id: Some(session_id.to_string()),
            logical_thread_id: Some(session_id.to_string()),
            source: SessionSourceState::LocalOnly,
            access: ProviderAccessState::BoundReady,
            title: format!("{provider} session"),
            preserved: None,
            summary: None,
            started_at,
            ended_at,
            last_prompt_at: None,
            transcript_path: None,
            working_directory: None,
            sync_blocked_reason: None,
            recorded_sha,
            is_active: false,
            user_round_count: 1,
            agent_message_count: 1,
            tool_call_count: 0,
            total_input_tokens: None,
            total_output_tokens: None,
            total_tokens: None,
            metrics_complete: true,
            actions: Vec::new(),
        }
    }

    #[test]
    fn every_provider_session_is_mapped_or_reported_unmapped() {
        let mut commits = vec![
            commit_at(300, &[]),
            commit_at(200, &[]),
            commit_at(100, &[]),
        ];
        let mut stored_only = session_at("claude", "claude-late", 310, 350, None);
        stored_only.source = SessionSourceState::RemoteOnly;
        stored_only.access = ProviderAccessState::ViewOnly;
        let sessions = vec![
            session_at("codex", "codex-1", 120, 150, None),
            session_at("claude", "claude-1", 220, 250, None),
            stored_only,
        ];

        let mapping = map_sessions_to_commits(&sessions, &mut commits);

        assert_eq!(
            commits[0].session_refs,
            vec![CommitSessionReference {
                resource_id: "claude:conversation:claude-1".to_string(),
            }]
        );
        assert_eq!(
            commits[1].session_refs,
            vec![CommitSessionReference {
                resource_id: "codex:conversation:codex-1".to_string(),
            }]
        );
        assert!(commits[2].session_refs.is_empty());
        assert_eq!(mapping.unique_session_count, 2);
        assert_eq!(
            mapping.unmapped,
            vec![UnmappedSessionReference {
                resource_id: "claude:conversation:claude-late".to_string(),
                reason: "No first-parent commit on this branch follows the session's last prompt"
                    .to_string(),
            }]
        );

        let represented = commits
            .iter()
            .flat_map(|commit| commit.session_refs.iter())
            .map(|reference| reference.resource_id.as_str())
            .chain(
                mapping
                    .unmapped
                    .iter()
                    .map(|reference| reference.resource_id.as_str()),
            )
            .collect::<BTreeSet<_>>();
        assert_eq!(represented.len(), sessions.len());
    }

    fn prompted(mut session: ProjectSessionSummary, last_prompt_at: u64) -> ProjectSessionSummary {
        session.last_prompt_at = Some(last_prompt_at);
        session
    }

    fn listed(commit: &GitCommitSummary) -> Vec<&str> {
        commit
            .session_refs
            .iter()
            .map(|reference| reference.resource_id.as_str())
            .collect()
    }

    #[test]
    fn a_session_maps_to_the_closest_commit_after_its_last_prompt_and_nowhere_else() {
        let at = |hours: u64, minutes: u64| hours * 3_600 + minutes * 60;
        let mut commits = vec![commit_at(at(21, 58), &[]), commit_at(at(20, 43), &[])];
        let sessions = vec![
            // Asked to commit at 20:42; the agent committed at 20:43 and
            // replied after it. The reply does not push it to the next commit.
            prompted(
                session_at("codex", "publish", at(20, 42), at(20, 44), None),
                at(20, 42),
            ),
            // Began before the first commit, last prompted after it.
            prompted(
                session_at("codex", "fable", at(20, 34), at(21, 47), None),
                at(21, 40),
            ),
            // Last prompted after every commit: uncommitted, and under no
            // commit it merely began before.
            prompted(
                session_at("claude", "label", at(20, 30), at(22, 6), None),
                at(22, 1),
            ),
        ];

        let mapping = map_sessions_to_commits(&sessions, &mut commits);

        assert_eq!(listed(&commits[0]), vec!["codex:conversation:fable"]);
        assert_eq!(listed(&commits[1]), vec!["codex:conversation:publish"]);
        assert_eq!(
            mapping
                .unmapped
                .iter()
                .map(|reference| reference.resource_id.as_str())
                .collect::<Vec<_>>(),
            vec!["claude:conversation:label"]
        );
        assert_eq!(mapping.unique_session_count, 2);
    }

    #[test]
    fn a_commit_in_the_second_of_the_last_prompt_takes_the_session() {
        let mut commits = vec![
            commit_at(300, &[]),
            commit_at(200, &[]),
            commit_at(100, &[]),
        ];

        map_sessions_to_commits(
            &[prompted(session_at("codex", "exact", 50, 250, None), 200)],
            &mut commits,
        );

        assert!(commits[0].session_refs.is_empty());
        assert_eq!(listed(&commits[1]), vec!["codex:conversation:exact"]);
        assert!(commits[2].session_refs.is_empty());
    }

    #[test]
    fn two_commits_in_one_second_resolve_to_the_older() {
        let mut newer = commit_at(200, &[]);
        newer.sha = "b".repeat(40);
        let mut commits = vec![newer, commit_at(200, &[]), commit_at(100, &[])];

        map_sessions_to_commits(
            &[prompted(session_at("codex", "tie", 120, 150, None), 150)],
            &mut commits,
        );

        assert!(commits[0].session_refs.is_empty());
        assert_eq!(listed(&commits[1]), vec!["codex:conversation:tie"]);
    }

    #[test]
    fn a_session_without_a_recorded_prompt_is_placed_by_its_end() {
        let mut commits = vec![
            commit_at(300, &[]),
            commit_at(200, &[]),
            commit_at(100, &[]),
        ];

        map_sessions_to_commits(
            &[session_at("codex", "silent", 50, 150, None)],
            &mut commits,
        );

        assert_eq!(listed(&commits[1]), vec!["codex:conversation:silent"]);
        assert!(commits[0].session_refs.is_empty());
        assert!(commits[2].session_refs.is_empty());
    }

    #[test]
    fn a_recorded_sha_is_provenance_and_never_moves_a_session() {
        let mut commits = vec![
            commit_at(300, &[]),
            commit_at(200, &[]),
            commit_at(100, &[]),
        ];
        let recorded = commits[0].sha.clone();

        let mapping = map_sessions_to_commits(
            &[prompted(
                session_at("codex", "recorded", 120, 150, Some(recorded)),
                130,
            )],
            &mut commits,
        );

        assert!(mapping.unmapped.is_empty());
        assert!(commits[0].session_refs.is_empty());
        assert_eq!(listed(&commits[1]), vec!["codex:conversation:recorded"]);
    }

    #[test]
    fn branch_discovery_lists_only_local_git_branches() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for args in [
            vec!["init", "--initial-branch=main"],
            vec!["config", "user.email", "mallard@example.test"],
            vec!["config", "user.name", "Mallard Test"],
        ] {
            let output = git_output(root, &args).unwrap();
            assert!(
                output.status.success(),
                "{}",
                git_error("set up test repository", &output)
            );
        }
        fs::write(root.join("tracked.txt"), "tracked\n").unwrap();
        for args in [vec!["add", "tracked.txt"], vec!["commit", "-m", "Initial"]] {
            let output = git_output(root, &args).unwrap();
            assert!(
                output.status.success(),
                "{}",
                git_error("commit test file", &output)
            );
        }
        let output = git_output(root, &["branch", "test"]).unwrap();
        assert!(
            output.status.success(),
            "{}",
            git_error("create test branch", &output)
        );

        let discovery = discover_git_branches(root, Some("test"))
            .unwrap()
            .expect("the temporary project is a Git repository");

        assert_eq!(discovery.selected_branch, "test");
        assert!(discovery.selected_has_ref);
        assert_eq!(
            discovery.branches,
            vec![
                GitBranchSummary {
                    name: "main".to_string(),
                    is_current: true,
                },
                GitBranchSummary {
                    name: "test".to_string(),
                    is_current: false,
                },
            ]
        );
        let error = match discover_git_branches(root, Some("recorded-only")) {
            Err(error) => error,
            Ok(_) => panic!("a non-local branch must be rejected"),
        };
        assert_eq!(
            error,
            "branch is not one of the repository's enumerated local branches"
        );
    }

    #[test]
    fn an_unborn_current_branch_remains_selectable_without_a_ref() {
        let temp = tempfile::tempdir().unwrap();
        let output = git_output(temp.path(), &["init", "--initial-branch=empty"]).unwrap();
        assert!(
            output.status.success(),
            "{}",
            git_error("initialize test repository", &output)
        );

        let discovery = discover_git_branches(temp.path(), None)
            .unwrap()
            .expect("the temporary project is a Git repository");

        assert_eq!(discovery.selected_branch, "empty");
        assert!(!discovery.selected_has_ref);
        assert_eq!(
            discovery.branches,
            vec![GitBranchSummary {
                name: "empty".to_string(),
                is_current: true,
            }]
        );
    }

    #[test]
    fn older_commits_offer_a_step_back_even_with_no_older_sessions() {
        // The defect this replaced: the cursor counted sessions alone, so a
        // project whose commits all predate the window had no way back to them.
        let older = older_activity(
            &[commit_at(500, &[]), commit_at(400, &[])],
            &[1_500, 1_800],
            1_000,
        )
        .expect("commits below the window are still something to step back to");
        assert_eq!(older.newest_at, 500);
        assert_eq!(older.commits, 2);
        assert_eq!(older.sessions, 0);
    }

    #[test]
    fn older_sessions_offer_a_step_back_even_with_no_older_commits() {
        let older = older_activity(&[commit_at(1_500, &[])], &[800, 600], 1_000)
            .expect("sessions below the window are something to step back to");
        assert_eq!(older.newest_at, 800);
        assert_eq!(older.commits, 0);
        assert_eq!(older.sessions, 2);
    }

    #[test]
    fn a_commit_held_on_the_page_by_its_session_is_not_also_waiting_below_it() {
        // An out-of-window commit still renders when a visible session maps to
        // it. Counting it again would promise a row that is already on screen,
        // and pointing the cursor at it would reload the same page forever.
        let older = older_activity(
            &[
                commit_at(900, &["codex:session:visible"]),
                commit_at(700, &[]),
            ],
            &[1_200],
            1_000,
        )
        .expect("the unclaimed commit still lies below");
        assert_eq!(older.newest_at, 700);
        assert_eq!(older.commits, 1);
    }

    #[test]
    fn the_cursor_takes_the_newest_of_either_kind() {
        assert_eq!(
            older_activity(&[commit_at(900, &[])], &[400], 1_000)
                .unwrap()
                .newest_at,
            900,
        );
        assert_eq!(
            older_activity(&[commit_at(400, &[])], &[900], 1_000)
                .unwrap()
                .newest_at,
            900,
        );
    }

    #[test]
    fn nothing_below_the_window_offers_no_step_at_all() {
        // The floor: absent `older`, the page states its range and draws no
        // control, rather than offering one that reloads the same rows.
        assert!(older_activity(&[commit_at(1_500, &[])], &[2_000], 1_000).is_none());
        assert!(older_activity(&[], &[], 1_000).is_none());
        // The window is half-open at the bottom, so its own start is inside it.
        assert!(older_activity(&[commit_at(1_000, &[])], &[1_000], 1_000).is_none());
    }

    struct InstrumentedAdapter {
        provider: ProviderKey,
        transcript: PathBuf,
        project_root: PathBuf,
        summary_calls: Arc<AtomicUsize>,
        timeline_calls: Arc<AtomicUsize>,
    }

    impl crate::providers::SessionAdapter for InstrumentedAdapter {
        fn descriptor(&self) -> ProviderDescriptor {
            ProviderDescriptor {
                key: self.provider.clone(),
                display_name: "Instrumented".to_string(),
                display_rank: 50,
                profile_label: "Instrumented profile".to_string(),
                default_profile_directory: None,
                setup_default: false,
                has_cli: false,
                maintenance_actions: Vec::new(),
            }
        }

        fn codec_key(&self) -> &str {
            "instrumented.v1"
        }

        fn codec_version(&self) -> u32 {
            1
        }

        fn discover(
            &self,
            _context: &SessionDiscoveryContext<'_>,
        ) -> Result<crate::providers::SessionDiscovery, String> {
            Ok(crate::providers::SessionDiscovery {
                sessions: vec![crate::providers::DiscoveredSession {
                    session_id: "session-1".to_string(),
                    physical_path: self.transcript.clone(),
                    display_name: "Session one (3)".to_string(),
                    blocked_reason: None,
                }],
                ..Default::default()
            })
        }

        fn scan_metadata(&self, _path: &Path) -> Result<SessionFileMetadata, String> {
            Ok(SessionFileMetadata {
                session_id: "session-1".to_string(),
                authorized_home: self.project_root.clone(),
                is_internal_subagent: false,
                is_empty: false,
            })
        }

        fn analyze(&self, _bytes: &[u8]) -> SessionAnalysis {
            panic!("Activity must not analyze transcript bytes")
        }

        fn analyze_and_summarize(
            &self,
            bytes: &[u8],
        ) -> (SessionAnalysis, Result<SessionFileSummary, String>) {
            (self.analyze(bytes), self.summarize_bytes(bytes))
        }

        fn materialization_targets(
            &self,
            _request: &MaterializationTargetRequest<'_>,
        ) -> Result<Vec<PlannedProviderTarget>, String> {
            panic!("Activity must not plan materialization")
        }

        fn summarize_file(&self, _path: &Path) -> Result<SessionFileSummary, String> {
            self.summary_calls.fetch_add(1, Ordering::SeqCst);
            Ok(SessionFileSummary {
                session_id: "session-1".to_string(),
                title: Some("One".to_string()),
                summary: None,
                started_at: Some(1),
                ended_at: Some(2),
                last_prompt_at: Some(1),
                branch: None,
                recorded_sha: None,
                user_rounds: 1,
                agent_messages: 1,
                tool_calls: 0,
                total_input_tokens: Some(100),
                total_output_tokens: Some(23),
                total_tokens: Some(123),
                metrics_complete: true,
                is_internal_subagent: false,
            })
        }

        fn summarize_bytes(&self, _bytes: &[u8]) -> Result<SessionFileSummary, String> {
            panic!("Activity must not summarize captured transcript bytes")
        }

        fn build_timeline_index(
            &self,
            _path: &Path,
            _context: &SessionTimelineContext<'_>,
        ) -> Result<SessionTimelineIndex, String> {
            self.timeline_calls.fetch_add(1, Ordering::SeqCst);
            Err("timeline must not run for Activity".to_string())
        }

        fn read_timeline_page(
            &self,
            _path: &Path,
            _index: &SessionTimelineIndex,
            _query: &SessionTimelineQuery,
            _context: &SessionTimelineContext<'_>,
        ) -> Result<SessionTimelinePageSlice, String> {
            self.timeline_calls.fetch_add(1, Ordering::SeqCst);
            Err("timeline must not run for Activity".to_string())
        }

        fn read_event_detail(
            &self,
            _path: &Path,
            _index: &SessionTimelineIndex,
            _event_id: &str,
            _context: &SessionTimelineContext<'_>,
        ) -> Result<SessionEventDetail, String> {
            self.timeline_calls.fetch_add(1, Ordering::SeqCst);
            Err("timeline must not run for Activity".to_string())
        }

        fn available_actions(
            &self,
            _context: &SessionActionContext<'_>,
        ) -> Vec<SessionActionDescriptor> {
            Vec::new()
        }

        fn execute_action(
            &self,
            _request: &mut SessionActionRequest<'_>,
        ) -> Result<SessionActionOutcome, String> {
            Err("instrumented adapter has no actions".to_string())
        }
    }

    /// A local Codex session with no conversation binding is found by its
    /// native file name through the alias rules, reading only its own
    /// rollout head; a thread no rollout claims is refused without a read.
    #[test]
    fn action_session_is_located_without_a_binding_from_its_own_rollout_alone() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let project = temp.path().join("project");
        fs::create_dir_all(home.join("sessions/2026/09/12")).unwrap();
        fs::create_dir_all(&project).unwrap();
        let home = fs::canonicalize(&home).unwrap();
        let project = fs::canonicalize(&project).unwrap();
        let day = home.join("sessions/2026/09/12");
        let wanted = "0193b6c1-5352-7b12-9c6e-000000000001";
        for id in [wanted, "0193b6c1-5352-7b12-9c6e-000000000002"] {
            let header = serde_json::json!({
                "type": "session_meta", "payload": {"id": id, "cwd": project}
            });
            let message = serde_json::json!({
                "type": "event_msg", "payload": {"type": "user_message", "message": "Hello"}
            });
            fs::write(
                day.join(format!("rollout-2026-09-12T01-03-07-{id}.jsonl")),
                format!("{header}\n{message}\n"),
            )
            .unwrap();
        }

        let (mut resolved, provider, profile_id) = alias_test_context();
        resolved.project_root = project.clone();
        resolved.binding.project_root = project.to_string_lossy().into_owned();
        resolved.binding.canonical_project_root = project.to_string_lossy().into_owned();
        resolved.homes.insert(
            provider.clone(),
            ResolvedProviderHome {
                profile_id: profile_id.clone(),
                home: home.clone(),
            },
        );
        let state = crate::persistence::LocalProjectActivityStateSnapshot {
            host_identity: mallard_domain::HostIdentity {
                schema: mallard_domain::HOST_IDENTITY_SCHEMA,
                revision: 0,
                host_id: resolved.binding.host_id.clone(),
                local_display_name: None,
                created_at: 1,
                updated_at: 1,
            },
            config: SyncConfig::default(),
            bindings: mallard_domain::MachineProjectState {
                schema: mallard_domain::MACHINE_PROJECT_SCHEMA,
                revision: 0,
                profiles: vec![ProviderProfile {
                    profile_id: profile_id.clone(),
                    provider: provider.clone(),
                    display_name: "Codex".to_string(),
                    path: home.to_string_lossy().into_owned(),
                    canonical_path: home.to_string_lossy().into_owned(),
                    provider_config: mallard_domain::AgentProviderConfig::version_1(),
                    revision: 0,
                    created_at: 1,
                    updated_at: 1,
                }],
                bindings: vec![resolved.binding.clone()],
            },
            conversation_bindings: ConversationBindings::empty(resolved.binding.host_id.clone()),
            materializations: Materializations::default(),
        };
        let registry = AgentProviderRegistry::from_adapters(vec![Arc::new(
            crate::providers::codex::CodexAdapter::new(),
        )])
        .unwrap();
        let resource = |provider_thread_id: &str| {
            let thread_id =
                initial_conversation_thread_id(&resolved.bundle_id, &provider, provider_thread_id)
                    .unwrap();
            (
                format!("codex:conversation:{}", thread_id.as_str()),
                thread_id,
            )
        };
        let metrics = crate::sync_metrics::register_root(&home);

        let (resource_id, thread_id) = resource(wanted);
        let before = metrics.snapshot();
        let (found_provider, found_thread, session) =
            resolve_action_session(&registry, &state, &resolved, &resource_id).unwrap();
        assert_eq!(found_provider, provider);
        assert_eq!(found_thread, thread_id);
        assert_eq!(session.session_id, wanted);
        assert_eq!(
            session.physical_path,
            day.join(format!("rollout-2026-09-12T01-03-07-{wanted}.jsonl"))
        );
        assert_eq!(metrics.snapshot().delta_since(&before).files_head_read, 1);

        let (resource_id, _) = resource("0193b6c1-5352-7b12-9c6e-000000000009");
        let before = metrics.snapshot();
        assert_eq!(
            resolve_action_session(&registry, &state, &resolved, &resource_id).unwrap_err(),
            "session does not belong to the selected project"
        );
        assert_eq!(metrics.snapshot().delta_since(&before).files_head_read, 0);
    }

    fn alias_test_context() -> (ResolvedActivityProject, ProviderKey, LocalProviderProfileId) {
        let provider = ProviderKey::parse("codex").unwrap();
        let profile_id = LocalProviderProfileId::parse("profile-alias-test").unwrap();
        let project_id = LocalProjectId::parse("project-alias-test").unwrap();
        let bundle_id = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let host_id = HostId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let replica_id = ReplicaId::parse("replica-alias-test").unwrap();
        let project_root = PathBuf::from("/tmp/mallard-activity-alias-test");
        let binding = ProjectBinding {
            host_id,
            replica_id,
            local_project_id: project_id.clone(),
            bundle_id: bundle_id.clone(),
            project_root: project_root.to_string_lossy().into_owned(),
            canonical_project_root: project_root.to_string_lossy().into_owned(),
            profile_ids: BTreeMap::from([(provider.clone(), profile_id.clone())]),
            state: BindingState::Active,
            revision: 0,
            updated_at: 1,
        };
        (
            ResolvedActivityProject {
                project_id,
                bundle_id,
                project_root,
                binding,
                homes: BTreeMap::new(),
                unavailable: BTreeMap::new(),
            },
            provider,
            profile_id,
        )
    }

    fn alias_test_binding(
        resolved: &ResolvedActivityProject,
        profile_id: &LocalProviderProfileId,
        thread_id: &ThreadId,
        provider_thread_id: &str,
    ) -> LocalConversationBinding {
        LocalConversationBinding {
            installed_end: None,
            installed_absent_subagents: Default::default(),
            pulled_children_excluded: false,
            installed_subagents: Default::default(),
            path_origins_ref: None,
            host_id: resolved.binding.host_id.clone(),
            replica_id: resolved.binding.replica_id.clone(),
            local_project_id: resolved.project_id.clone(),
            provider_profile_id: profile_id.clone(),
            thread_id: thread_id.clone(),
            cloud_thread_id: None,
            forked_from: None,
            preserved: None,
            root_tail_layout: None,
            provider_thread_id: Some(provider_thread_id.to_string()),
            materialization_state: ConversationMaterializationState::Resumable,
            revision: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn alias_test_receipt(
        resolved: &ResolvedActivityProject,
        profile_id: &LocalProviderProfileId,
        provider: &ProviderKey,
        thread_id: &ThreadId,
        resource_thread_id: &ThreadId,
        provider_thread_id: &str,
        suffix: &str,
        status: ActionStatus,
        materialization_state: ConversationMaterializationState,
    ) -> ApplyReceipt {
        ApplyReceipt {
            action_id: ActionId::parse(format!("action-{suffix}")).unwrap(),
            resource_id: conversation_resource_id(provider, resource_thread_id).unwrap(),
            action_type: RestoreActionType::MaterializeConversation,
            logical_path: None,
            source_sha256: None,
            target_path: None,
            target_sha256_after: None,
            status,
            applied_at: 1,
            error: None,
            conversation: Some(ConversationApplyReceipt {
                host_id: resolved.binding.host_id.clone(),
                replica_id: resolved.binding.replica_id.clone(),
                thread_id: thread_id.clone(),
                provider_profile_id: profile_id.clone(),
                provider_thread_id: Some(provider_thread_id.to_string()),
                materialization_state,
                lineage_digest: None,
                semantic_digest: None,
                local_file_sha256_after: None,
                cloud_revision: 1,
                cloud_commit_id: format!("commit-{suffix}"),
                cloud_manifest_sha256: "a".repeat(64),
                published_origin_host_token: None,
                published_origin_replica_token: None,
                operation_id: ConversationOperationId::parse(format!("operation-{suffix}"))
                    .unwrap(),
                accepted_tail: None,
                preserved_thread_id: None,
                preserved_display_title: None,
                source_disposition: None,
            }),
        }
    }

    fn alias_test_materializations(
        resolved: &ResolvedActivityProject,
        receipts: Vec<ApplyReceipt>,
        status: MaterializationStatus,
    ) -> Materializations {
        let mut materializations = Materializations::default();
        materializations.records.push(MaterializationRecord {
            materialization_id: MaterializationId::parse("materialization-alias-test").unwrap(),
            plan_id: PlanId::parse("plan-alias-test").unwrap(),
            host_id: resolved.binding.host_id.clone(),
            replica_id: resolved.binding.replica_id.clone(),
            local_project_id: resolved.project_id.clone(),
            storage_id: StorageId::parse("storage-alias-test").unwrap(),
            bundle_id: resolved.bundle_id.clone(),
            revision: 1,
            commit_id: "commit-alias-test".to_string(),
            manifest_sha256: "b".repeat(64),
            binding_revision: 0,
            status,
            applied_at: 1,
            receipts,
        });
        materializations
    }

    #[test]
    fn activity_exposes_displaced_native_alias_without_an_exact_byte_witness() {
        let (resolved, provider, profile_id) = alias_test_context();
        let old_alias = "native-old";
        let current_alias = "native-current";
        let thread_id =
            initial_conversation_thread_id(&resolved.bundle_id, &provider, old_alias).unwrap();
        let mut bindings = ConversationBindings::empty(resolved.binding.host_id.clone());
        bindings.bindings.push(alias_test_binding(
            &resolved,
            &profile_id,
            &thread_id,
            current_alias,
        ));
        let aliases =
            activity_conversation_aliases(&resolved, &bindings, &Materializations::default());
        let retained =
            retained_conversation_thread_id(&resolved.bundle_id, &provider, &thread_id, old_alias)
                .unwrap();

        assert_eq!(
            activity_thread_for_provider_alias(
                &resolved,
                &provider,
                old_alias,
                Path::new("/tmp/mallard-retained-native-old.jsonl"),
                &aliases,
                &SyncConfig::default(),
            )
            .unwrap(),
            Some(retained)
        );
        assert_eq!(
            activity_thread_for_provider_alias(
                &resolved,
                &provider,
                current_alias,
                Path::new("/tmp/mallard-retained-native-current.jsonl"),
                &aliases,
                &SyncConfig::default(),
            )
            .unwrap(),
            Some(thread_id)
        );
    }

    #[test]
    fn activity_uses_applied_receipt_owners_for_retained_branch_identity() {
        let (resolved, provider, profile_id) = alias_test_context();
        let imported_thread = ThreadId::parse("imported-thread").unwrap();
        let unrelated_thread = ThreadId::parse("unrelated-thread").unwrap();
        let current_alias = "alias-current";
        let mut bindings = ConversationBindings::empty(resolved.binding.host_id.clone());
        bindings.bindings.push(alias_test_binding(
            &resolved,
            &profile_id,
            &imported_thread,
            current_alias,
        ));
        let receipts = vec![
            alias_test_receipt(
                &resolved,
                &profile_id,
                &provider,
                &imported_thread,
                &imported_thread,
                "alias-old",
                "old-owner-moved",
                ActionStatus::Applied,
                ConversationMaterializationState::Resumable,
            ),
            // A later second owner must not erase the earlier owner from the
            // alias ledger. Any owner that now binds elsewhere displaces it.
            alias_test_receipt(
                &resolved,
                &profile_id,
                &provider,
                &unrelated_thread,
                &unrelated_thread,
                "alias-old",
                "old-owner-unbound",
                ActionStatus::Applied,
                ConversationMaterializationState::Resumable,
            ),
            alias_test_receipt(
                &resolved,
                &profile_id,
                &provider,
                &imported_thread,
                &imported_thread,
                "alias-middle",
                "middle-owner-moved",
                ActionStatus::Applied,
                ConversationMaterializationState::Resumable,
            ),
        ];
        // Partial records still contain durable applied receipts; record
        // completion is not an alias-history condition.
        let materializations =
            alias_test_materializations(&resolved, receipts, MaterializationStatus::Partial);
        let aliases = activity_conversation_aliases(&resolved, &bindings, &materializations);

        for displaced in ["alias-old", "alias-middle"] {
            let retained = retained_conversation_thread_id(
                &resolved.bundle_id,
                &provider,
                &imported_thread,
                displaced,
            )
            .unwrap();
            assert_eq!(
                activity_thread_for_provider_alias(
                    &resolved,
                    &provider,
                    displaced,
                    Path::new("/tmp/mallard-retained-imported.jsonl"),
                    &aliases,
                    &SyncConfig::default(),
                )
                .unwrap(),
                Some(retained)
            );
        }
        assert_eq!(
            activity_thread_for_provider_alias(
                &resolved,
                &provider,
                current_alias,
                Path::new("/tmp/mallard-retained-imported-current.jsonl"),
                &aliases,
                &SyncConfig::default(),
            )
            .unwrap(),
            Some(imported_thread)
        );
    }

    #[test]
    fn activity_current_binding_wins_over_another_threads_alias_history() {
        let (resolved, provider, profile_id) = alias_test_context();
        let historical_owner = ThreadId::parse("historical-owner").unwrap();
        let current_owner = ThreadId::parse("current-owner").unwrap();
        let mut bindings = ConversationBindings::empty(resolved.binding.host_id.clone());
        bindings.bindings.extend([
            alias_test_binding(
                &resolved,
                &profile_id,
                &historical_owner,
                "historical-owner-current",
            ),
            alias_test_binding(&resolved, &profile_id, &current_owner, "reused-alias"),
        ]);
        let materializations = alias_test_materializations(
            &resolved,
            vec![alias_test_receipt(
                &resolved,
                &profile_id,
                &provider,
                &historical_owner,
                &historical_owner,
                "reused-alias",
                "reused-alias",
                ActionStatus::Applied,
                ConversationMaterializationState::Resumable,
            )],
            MaterializationStatus::Complete,
        );
        let aliases = activity_conversation_aliases(&resolved, &bindings, &materializations);

        assert_eq!(
            activity_thread_for_provider_alias(
                &resolved,
                &provider,
                "reused-alias",
                Path::new("/tmp/mallard-retained-reused.jsonl"),
                &aliases,
                &SyncConfig::default(),
            )
            .unwrap(),
            Some(current_owner)
        );
    }

    #[test]
    fn activity_ignores_receipts_without_exact_applied_resumable_identity() {
        let (resolved, provider, profile_id) = alias_test_context();
        let target = ThreadId::parse("filter-target").unwrap();
        let mismatched = ThreadId::parse("filter-mismatch").unwrap();
        let other_profile = LocalProviderProfileId::parse("profile-filter-other").unwrap();
        let other_provider = ProviderKey::parse("claude").unwrap();
        let mut bindings = ConversationBindings::empty(resolved.binding.host_id.clone());
        bindings.bindings.push(alias_test_binding(
            &resolved,
            &profile_id,
            &target,
            "filter-current",
        ));
        let receipts = vec![
            alias_test_receipt(
                &resolved,
                &profile_id,
                &provider,
                &target,
                &target,
                "skipped-alias",
                "filter-skipped",
                ActionStatus::Skipped,
                ConversationMaterializationState::Resumable,
            ),
            alias_test_receipt(
                &resolved,
                &profile_id,
                &provider,
                &target,
                &target,
                "pending-alias",
                "filter-pending",
                ActionStatus::Applied,
                ConversationMaterializationState::Pending,
            ),
            alias_test_receipt(
                &resolved,
                &profile_id,
                &provider,
                &target,
                &mismatched,
                "logical-mismatch-alias",
                "filter-logical",
                ActionStatus::Applied,
                ConversationMaterializationState::Resumable,
            ),
            alias_test_receipt(
                &resolved,
                &other_profile,
                &provider,
                &target,
                &target,
                "profile-mismatch-alias",
                "filter-profile",
                ActionStatus::Applied,
                ConversationMaterializationState::Resumable,
            ),
            alias_test_receipt(
                &resolved,
                &profile_id,
                &other_provider,
                &target,
                &target,
                "provider-mismatch-alias",
                "filter-provider",
                ActionStatus::Applied,
                ConversationMaterializationState::Resumable,
            ),
        ];
        let materializations =
            alias_test_materializations(&resolved, receipts, MaterializationStatus::Complete);
        let aliases = activity_conversation_aliases(&resolved, &bindings, &materializations);

        for retained in [
            "skipped-alias",
            "pending-alias",
            "logical-mismatch-alias",
            "profile-mismatch-alias",
            "provider-mismatch-alias",
        ] {
            assert_eq!(
                activity_thread_for_provider_alias(
                    &resolved,
                    &provider,
                    retained,
                    Path::new("/tmp/mallard-retained-filter.jsonl"),
                    &aliases,
                    &SyncConfig::default(),
                )
                .unwrap(),
                Some(
                    initial_conversation_thread_id(&resolved.bundle_id, &provider, retained)
                        .unwrap()
                )
            );
        }
    }

    #[test]
    fn activity_cache_keeps_other_projects_in_a_shared_provider_home() {
        for provider_name in ["codex", "claude"] {
            let temporary = tempfile::tempdir().unwrap();
            let repository = LocalStateRepository::from_home_dir(temporary.path()).unwrap();
            let home = temporary.path().join("provider");
            fs::create_dir(&home).unwrap();
            let provider = ProviderKey::parse(provider_name).unwrap();
            let make_project = |suffix: &str| {
                let (mut resolved, _, profile_id) = alias_test_context();
                resolved.project_id = LocalProjectId::parse(format!("project-{suffix}")).unwrap();
                resolved.project_root = temporary.path().join(suffix);
                fs::create_dir(&resolved.project_root).unwrap();
                resolved.binding.local_project_id = resolved.project_id.clone();
                resolved.binding.project_root =
                    resolved.project_root.to_string_lossy().into_owned();
                resolved.binding.canonical_project_root = resolved.binding.project_root.clone();
                resolved.binding.profile_ids =
                    BTreeMap::from([(provider.clone(), profile_id.clone())]);
                resolved.homes.insert(
                    provider.clone(),
                    ResolvedProviderHome {
                        profile_id,
                        home: home.clone(),
                    },
                );
                let source_path = home.join(format!("{suffix}.jsonl"));
                fs::write(&source_path, b"{}\n").unwrap();
                let metadata = fs::metadata(&source_path).unwrap();
                let view = LocalSessionView {
                    descriptor: crate::provider_capture::ResourceCandidate {
                        resource_id: format!("{provider}:session:session-1"),
                        kind: mallard_domain::portable::contracts::conversation(),
                        provider: Some(provider.clone()),
                        provider_resource_type: Some("session".to_string()),
                        display_name: "Session one".to_string(),
                        apply_policy: mallard_domain::portable::contracts::safe_file(),
                        selected_by_default: true,
                        blocked_reason: None,
                        logical_paths: vec!["state/session.jsonl".to_string()],
                        project_content: None,
                    },
                    provider: provider.clone(),
                    provider_thread_id: "session-1".to_string(),
                    transcript_logical_path: "state/session.jsonl".to_string(),
                    source_path,
                    source_mtime: modified_secs(&metadata),
                    source_size: metadata.len(),
                    capture_bytes: metadata.len(),
                };
                (resolved, vec![view])
            };
            let (project_a, views_a) = make_project("a");
            let (project_b, views_b) = make_project("ab");
            let summary_calls = Arc::new(AtomicUsize::new(0));
            let registry =
                AgentProviderRegistry::from_adapters(vec![Arc::new(InstrumentedAdapter {
                    provider,
                    transcript: views_a[0].source_path.clone(),
                    project_root: project_a.project_root.clone(),
                    summary_calls: summary_calls.clone(),
                    timeline_calls: Arc::new(AtomicUsize::new(0)),
                })])
                .unwrap();
            let mut warnings = Vec::new();
            for (project, views, expected_calls, expected_entries) in [
                (&project_a, &views_a, 1, 1),
                (&project_b, &views_b, 2, 2),
                (&project_a, &views_a, 2, 2),
            ] {
                let rows =
                    scan_local_session_views(&repository, &registry, project, views, &mut warnings)
                        .unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(summary_calls.load(Ordering::SeqCst), expected_calls);
                let (cache, _) = load_activity_cache(
                    repository.root(),
                    &repository.root().join("activity_cache.json"),
                    &mut warnings,
                );
                assert_eq!(cache.entries.len(), expected_entries);
            }

            // Remove A's stale entry, then confirm B still uses its cached summary.
            scan_local_session_views(&repository, &registry, &project_a, &[], &mut warnings)
                .unwrap();
            let (cache, _) = load_activity_cache(
                repository.root(),
                &repository.root().join("activity_cache.json"),
                &mut warnings,
            );
            assert_eq!(cache.entries.len(), 1);
            assert!(cache
                .entries
                .keys()
                .all(|key| key_belongs_to_project(key, &project_b)));
            let rows = scan_local_session_views(
                &repository,
                &registry,
                &project_b,
                &views_b,
                &mut warnings,
            )
            .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(summary_calls.load(Ordering::SeqCst), 2);
            assert!(warnings.is_empty(), "{warnings:?}");
        }
    }

    #[test]
    fn activity_cache_fingerprints_control_summary_reuse() {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path().join("provider");
        let project_root = temporary.path().join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&project_root).unwrap();
        let transcript = home.join("session.jsonl");
        fs::write(&transcript, b"{}\n").unwrap();
        let repository = LocalStateRepository::from_home_dir(temporary.path()).unwrap();
        let provider = ProviderKey::parse("instrumented").unwrap();
        let profile_id = LocalProviderProfileId::parse("profile-test").unwrap();
        let project_id = LocalProjectId::parse("project-test").unwrap();
        let bundle_id = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let identity = repository.load_or_create_host_identity().unwrap();
        let canonical_project_root = fs::canonicalize(&project_root).unwrap();
        let canonical_home = fs::canonicalize(&home).unwrap();
        let project_binding = ProjectBinding {
            host_id: identity.host_id.clone(),
            replica_id: ReplicaId::parse("replica-test").unwrap(),
            local_project_id: project_id.clone(),
            bundle_id: bundle_id.clone(),
            project_root: project_root.to_string_lossy().into_owned(),
            canonical_project_root: canonical_project_root.to_string_lossy().into_owned(),
            profile_ids: BTreeMap::from([(provider.clone(), profile_id.clone())]),
            state: BindingState::Active,
            revision: 0,
            updated_at: 1,
        };
        repository
            .mutate_config(|config| {
                config.projects.push(LocalProjectRegistration {
                    local_project_id: project_id.clone(),
                    bundle_id: bundle_id.clone(),
                    display_name: "Test project".to_string(),
                    local_alias: None,
                    repository_fingerprint: None,
                    recipe: BundleRecipe::default(),
                    revision: 0,
                    created_at: 1,
                    updated_at: 1,
                });
                Ok(())
            })
            .unwrap();
        repository
            .mutate_bindings(|_, machine| {
                machine.profiles.push(ProviderProfile {
                    profile_id: profile_id.clone(),
                    provider: provider.clone(),
                    display_name: "Instrumented".to_string(),
                    path: home.to_string_lossy().into_owned(),
                    canonical_path: canonical_home.to_string_lossy().into_owned(),
                    provider_config: mallard_domain::AgentProviderConfig::version_1(),
                    revision: 0,
                    created_at: 1,
                    updated_at: 1,
                });
                machine.bindings.push(project_binding.clone());
                Ok(())
            })
            .unwrap();
        let summary_calls = Arc::new(AtomicUsize::new(0));
        let timeline_calls = Arc::new(AtomicUsize::new(0));
        let registry = AgentProviderRegistry::from_adapters(vec![Arc::new(InstrumentedAdapter {
            provider: provider.clone(),
            transcript: transcript.clone(),
            project_root: project_root.clone(),
            summary_calls: summary_calls.clone(),
            timeline_calls: timeline_calls.clone(),
        })])
        .unwrap();
        let resolved = ResolvedActivityProject {
            project_id: project_id.clone(),
            bundle_id: bundle_id.clone(),
            project_root: project_root.clone(),
            binding: project_binding,
            homes: BTreeMap::from([(
                provider.clone(),
                ResolvedProviderHome {
                    profile_id,
                    home: canonical_home,
                },
            )]),
            unavailable: BTreeMap::new(),
        };
        let mut warnings = Vec::new();
        let transcript_metadata = fs::metadata(&transcript).unwrap();
        let views = vec![LocalSessionView {
            descriptor: crate::provider_capture::ResourceCandidate {
                resource_id: "instrumented:session:session-1".to_string(),
                kind: mallard_domain::portable::contracts::conversation(),
                provider: Some(provider.clone()),
                provider_resource_type: Some("session".to_string()),
                display_name: "Session one (3)".to_string(),
                apply_policy: mallard_domain::portable::contracts::safe_file(),
                selected_by_default: true,
                blocked_reason: None,
                logical_paths: vec!["state/session.jsonl".to_string()],
                project_content: None,
            },
            provider: provider.clone(),
            provider_thread_id: "session-1".to_string(),
            transcript_logical_path: "state/session.jsonl".to_string(),
            source_path: transcript.clone(),
            source_mtime: modified_secs(&transcript_metadata),
            source_size: transcript_metadata.len(),
            capture_bytes: transcript_metadata.len(),
        }];
        let rows =
            scan_local_session_views(&repository, &registry, &resolved, &views, &mut warnings)
                .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(summary_calls.load(Ordering::SeqCst), 1);

        let cache_path = repository.root().join("activity_cache.json");
        let cache_timestamp = UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        let cache_file = fs::File::options().write(true).open(&cache_path).unwrap();
        cache_file.set_modified(cache_timestamp).unwrap();
        drop(cache_file);
        let cached_rows =
            scan_local_session_views(&repository, &registry, &resolved, &views, &mut warnings)
                .unwrap();
        assert_eq!(cached_rows[0].summary, rows[0].summary);
        assert_eq!(summary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            fs::metadata(&cache_path).unwrap().modified().unwrap(),
            cache_timestamp,
            "a warm cache hit must not rewrite the cache"
        );

        fs::write(&transcript, b"{}\n{}\n").unwrap();
        let size_after_append = fs::metadata(&transcript).unwrap().len();
        assert_ne!(size_after_append, views[0].source_size);
        let appended_rows =
            scan_local_session_views(&repository, &registry, &resolved, &views, &mut warnings)
                .unwrap();
        assert_eq!(appended_rows[0].summary, rows[0].summary);
        assert_eq!(summary_calls.load(Ordering::SeqCst), 2);
        assert_ne!(
            fs::metadata(&cache_path).unwrap().modified().unwrap(),
            cache_timestamp,
            "an updated summary must rewrite the cache"
        );

        let mtime_before_rewrite = modified_nanos(&fs::metadata(&transcript).unwrap());
        fs::write(&transcript, b"[]\n{}\n").unwrap();
        let transcript_file = fs::File::options().write(true).open(&transcript).unwrap();
        transcript_file
            .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1_900_000_000))
            .unwrap();
        let rewritten_metadata = fs::metadata(&transcript).unwrap();
        assert_eq!(rewritten_metadata.len(), size_after_append);
        assert_ne!(rewritten_metadata.len(), views[0].source_size);
        assert_ne!(modified_secs(&rewritten_metadata), views[0].source_mtime);
        assert_ne!(modified_nanos(&rewritten_metadata), mtime_before_rewrite);
        let rewritten_rows =
            scan_local_session_views(&repository, &registry, &resolved, &views, &mut warnings)
                .unwrap();
        assert_eq!(rewritten_rows[0].summary, rows[0].summary);
        assert_eq!(summary_calls.load(Ordering::SeqCst), 3);

        let preserved_thread_id = initial_conversation_thread_id(
            &resolved.bundle_id,
            &ProviderKey::parse("instrumented").unwrap(),
            "session-1",
        )
        .unwrap();
        repository
            .mutate_conversation_bindings(|bindings| {
                bindings.bindings.push(LocalConversationBinding {
                    installed_end: None,
                    installed_absent_subagents: Default::default(),
                    pulled_children_excluded: false,
                    installed_subagents: Default::default(),
                    path_origins_ref: None,
                    host_id: resolved.binding.host_id.clone(),
                    replica_id: resolved.binding.replica_id.clone(),
                    local_project_id: resolved.project_id.clone(),
                    provider_profile_id: LocalProviderProfileId::parse("profile-test").unwrap(),
                    thread_id: preserved_thread_id.clone(),
                    cloud_thread_id: None,
                    forked_from: Some(CloudThreadId::parse("cloudthread-parent").unwrap()),
                    preserved: None,
                    root_tail_layout: None,
                    provider_thread_id: Some("session-1".to_string()),
                    materialization_state: ConversationMaterializationState::Resumable,
                    revision: 0,
                    created_at: 1,
                    updated_at: 1,
                });
                Ok(())
            })
            .unwrap();
        let sessions = merge_activity_sessions(
            &registry,
            &repository,
            &resolved,
            rows.clone(),
            Vec::new(),
            &mut warnings,
        )
        .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Session one (3)");
        assert_eq!(sessions[0].total_input_tokens, Some(100));
        assert_eq!(sessions[0].total_output_tokens, Some(23));
        assert_eq!(sessions[0].total_tokens, Some(123));
        assert_eq!(
            sessions[0].transcript_path.as_deref().map(Path::new),
            Some(transcript.as_path())
        );
        assert!(sessions[0].preserved.is_none());

        repository
            .mutate_conversation_bindings(|bindings| {
                let binding = bindings
                    .bindings
                    .iter_mut()
                    .find(|binding| binding.thread_id == preserved_thread_id)
                    .unwrap();
                binding.preserved = Some(PreservedProvenance {
                    preserved_from: CloudThreadId::parse("cloudthread-parent").unwrap(),
                    ordinal: 2,
                });
                binding.revision = binding.revision.saturating_add(1);
                binding.updated_at = 2;
                Ok(())
            })
            .unwrap();
        assert!(repository
            .load_materializations()
            .unwrap()
            .records
            .is_empty());
        let preserved_sessions = merge_activity_sessions(
            &registry,
            &repository,
            &resolved,
            rows,
            Vec::new(),
            &mut warnings,
        )
        .unwrap();
        assert_eq!(
            preserved_sessions[0].preserved,
            Some(PreservedProvenance {
                preserved_from: CloudThreadId::parse("cloudthread-parent").unwrap(),
                ordinal: 2,
            })
        );

        let remote_only = merge_activity_sessions(
            &registry,
            &repository,
            &resolved,
            Vec::new(),
            vec![RemoteSessionRecord {
                provider,
                thread_id: CloudThreadId::parse("cloudthread-storage-only").unwrap(),
                source_thread_id: Some("provider-storage-only".to_string()),
                display_title: Some("Storage-only session".to_string()),
                preserved: None,
                presentation: mallard_domain::SessionPresentation {
                    metrics: mallard_domain::SessionMetrics {
                        total_input_tokens: Some(200),
                        total_output_tokens: Some(50),
                        total_tokens: Some(250),
                        complete: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            }],
            &mut warnings,
        )
        .unwrap();
        assert_eq!(remote_only.len(), 1);
        assert_eq!(remote_only[0].total_input_tokens, Some(200));
        assert_eq!(remote_only[0].total_output_tokens, Some(50));
        assert_eq!(remote_only[0].total_tokens, Some(250));
        assert!(remote_only[0].transcript_path.is_none());
        assert!(serde_json::to_value(&remote_only[0])
            .unwrap()
            .get("transcript_path")
            .is_none());
        assert_eq!(summary_calls.load(Ordering::SeqCst), 3);
        assert_eq!(timeline_calls.load(Ordering::SeqCst), 0);

        let cache_file = fs::File::options().write(true).open(&cache_path).unwrap();
        cache_file.set_modified(cache_timestamp).unwrap();
        drop(cache_file);
        let removed_rows =
            scan_local_session_views(&repository, &registry, &resolved, &[], &mut warnings)
                .unwrap();
        assert!(removed_rows.is_empty());
        assert_ne!(
            fs::metadata(&cache_path).unwrap().modified().unwrap(),
            cache_timestamp,
            "retained-away entries must rewrite the cache"
        );
        let (cache, cache_changed) =
            load_activity_cache(repository.root(), &cache_path, &mut warnings);
        assert!(!cache_changed);
        assert!(cache.entries.is_empty());
    }
}
