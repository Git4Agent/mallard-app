//! Codex session discovery under one `CODEX_HOME`.

use super::filter_codex_session_index_by_ids;
use crate::provider_capture::{
    canonical_existing_dir, inspect_candidate_files_with_limit, modified_secs, provider_walk_entry,
    session_home_is_project_root,
};
use crate::providers::codex::history::parse_session_index_titles;
use crate::providers::traits::{
    DiscoveredSession, SessionDiscovery, SessionDiscoveryContext, SessionHomeSighting,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MAX_PROFILE_SESSION_FILES: usize = 20_000;
const MAX_PROFILE_SESSION_ENTRIES: usize = 80_000;
/// How many of the newest rollouts `recent_session_homes` opens. Each is a
/// head read; 500 measured 0.28 s on a 2,469-rollout home and named every
/// folder in recent use.
const RECENT_SESSION_FILES: usize = 500;

/// The folders the newest rollouts ran in. Rollouts are ordered by
/// modification time from one walk — a stat each — and only the newest
/// [`RECENT_SESSION_FILES`] are opened. A walk past its entry bound stops
/// there rather than failing: this is a hint, not an inventory.
pub(super) fn recent_session_homes(
    provider_home: &Path,
    scan_metadata: impl Fn(&Path) -> Result<crate::providers::traits::SessionFileMetadata, String>,
) -> Result<Vec<SessionHomeSighting>, String> {
    let canonical_home = canonical_existing_dir(provider_home, "Codex home")?;
    let mut rollouts: Vec<(u64, PathBuf)> = Vec::new();
    let mut visited_entries = 0_usize;
    'walk: for directory in ["sessions", "archived_sessions"] {
        let state_root = canonical_home.join(directory);
        if !state_root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&state_root)
            .follow_links(false)
            .max_depth(8)
            .into_iter()
            .filter_entry(provider_walk_entry)
        {
            let Ok(entry) = entry else {
                continue;
            };
            visited_entries = visited_entries.saturating_add(1);
            if visited_entries > MAX_PROFILE_SESSION_ENTRIES {
                break 'walk;
            }
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            if let Ok(metadata) = entry.metadata() {
                rollouts.push((modified_secs(&metadata), entry.into_path()));
            }
        }
    }
    rollouts.sort_by(|left, right| right.cmp(left));
    let mut homes: BTreeMap<PathBuf, u64> = BTreeMap::new();
    for (modified, path) in rollouts.into_iter().take(RECENT_SESSION_FILES) {
        let Ok(metadata) = scan_metadata(&path) else {
            continue;
        };
        // A subagent runs where its parent does; the parent names the folder.
        if metadata.is_internal_subagent {
            continue;
        }
        let last_active = homes.entry(metadata.authorized_home).or_insert(0);
        *last_active = (*last_active).max(modified);
    }
    Ok(homes
        .into_iter()
        .map(|(home, last_active)| SessionHomeSighting { home, last_active })
        .collect())
}

pub(super) fn discover(
    context: &SessionDiscoveryContext<'_>,
    scan_metadata: impl Fn(
        &std::path::Path,
    ) -> Result<crate::providers::traits::SessionFileMetadata, String>,
) -> Result<SessionDiscovery, String> {
    let canonical_home = canonical_existing_dir(context.provider_home, "Codex home")?;
    let mut discovery = SessionDiscovery::default();
    let mut selected_thread_ids = BTreeSet::new();
    let mut by_session: BTreeMap<String, usize> = BTreeMap::new();
    let mut candidates: BTreeMap<String, Vec<std::path::PathBuf>> = BTreeMap::new();
    let mut metadata = BTreeMap::new();
    let mut canonical_homes = BTreeMap::new();
    let mut visited_entries = 0_usize;
    let mut visited_files = 0_usize;
    for directory in ["sessions", "archived_sessions"] {
        let state_root = canonical_home.join(directory);
        if !state_root.exists() {
            continue;
        }
        for entry in WalkDir::new(&state_root)
            .follow_links(false)
            .max_depth(8)
            .into_iter()
            .filter_entry(provider_walk_entry)
        {
            let entry = entry.map_err(|e| format!("walk Codex {}: {}", directory, e))?;
            visited_entries = visited_entries.saturating_add(1);
            if visited_entries > MAX_PROFILE_SESSION_ENTRIES {
                return Err("Codex profile session inventory exceeds its entry bound".to_string());
            }
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            visited_files = visited_files.saturating_add(1);
            if visited_files > MAX_PROFILE_SESSION_FILES {
                return Err(format!(
                    "Codex profile contains more than {} session files",
                    MAX_PROFILE_SESSION_FILES
                ));
            }
            let meta = match scan_metadata(entry.path()) {
                Ok(meta) => meta,
                Err(error) => {
                    discovery.warnings.push(format!(
                        "ignored Codex session '{}': {}",
                        entry.path().display(),
                        error
                    ));
                    continue;
                }
            };
            candidates
                .entry(meta.session_id.clone())
                .or_default()
                .push(entry.path().to_path_buf());
            metadata.insert(entry.path().to_path_buf(), meta);
        }
    }
    let lookup = crate::codex_subagents::RolloutLookup::new(&canonical_home);
    for (session_id, mut paths) in candidates {
        paths.sort();
        let (physical_path, selection_error) = match lookup.select_current(&session_id, &paths) {
            Ok(Some(path)) => (path, None),
            Ok(None) => continue,
            Err(error) => (paths[0].clone(), Some(error)),
        };
        let meta = &metadata[&physical_path];
        if !session_home_is_project_root(
            context.project_root,
            &meta.authorized_home,
            &mut canonical_homes,
        ) {
            continue;
        }
        if meta.is_internal_subagent {
            discovery.excluded_subagent_session_ids.insert(session_id);
            continue;
        }
        selected_thread_ids.insert(session_id.clone());
        by_session.insert(session_id.clone(), discovery.sessions.len());
        let blocked_reason = selection_error.or_else(|| {
            inspect_candidate_files_with_limit(
                &canonical_home,
                std::slice::from_ref(&physical_path),
                crate::app_settings::conversation_file_limit_bytes(),
            )
            .err()
        });
        discovery.sessions.push(DiscoveredSession {
            display_name: session_id.clone(),
            session_id,
            physical_path,
            blocked_reason,
        });
    }

    let index = canonical_home.join("session_index.jsonl");
    if !selected_thread_ids.is_empty() && index.is_file() {
        // The index is local provider state, not a portable resource; read
        // only the selected rows to enrich the typed conversation titles.
        let filtered = filter_codex_session_index_by_ids(&index, &selected_thread_ids)?;
        if !filtered.is_empty() {
            for (thread_id, title) in parse_session_index_titles(&filtered) {
                if let Some(position) = by_session.get(&thread_id) {
                    discovery.sessions[*position].display_name = title;
                }
            }
        }
    }
    Ok(discovery)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::codex::CodexAdapter;
    use crate::providers::traits::SessionAdapter;
    use std::time::{Duration, SystemTime};

    fn write_rollout(
        directory: &Path,
        name: &str,
        cwd: &Path,
        subagent: bool,
        modified: SystemTime,
    ) {
        let mut payload = serde_json::json!({ "id": name, "cwd": cwd });
        if subagent {
            payload["thread_source"] = serde_json::json!("subagent");
        }
        let record = serde_json::json!({ "type": "session_meta", "payload": payload });
        let path = directory.join(format!("rollout-{name}.jsonl"));
        std::fs::write(&path, format!("{record}\n")).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }

    /// Only the newest rollouts are opened: a folder named only by older
    /// ones is not offered, and a subagent never names a folder.
    #[test]
    fn recent_session_homes_reads_only_the_newest_rollouts() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let day = home.join("sessions/2026/09/25");
        std::fs::create_dir_all(&day).unwrap();
        let recent = temp.path().join("work/recent");
        let old = temp.path().join("work/old");
        // The clock is read once. Every file time and the expected time
        // come from this reading, however long the test takes.
        let now = SystemTime::now();
        let aged = |secs: u64| now - Duration::from_secs(secs);
        for index in 0..RECENT_SESSION_FILES {
            write_rollout(
                &day,
                &format!("recent-{index}"),
                &recent,
                false,
                aged(100 + index as u64),
            );
        }
        write_rollout(&day, "old", &old, false, aged(10_000));
        write_rollout(
            &day,
            "child",
            &temp.path().join("work/child"),
            true,
            aged(10),
        );

        let homes = CodexAdapter::new().recent_session_homes(&home).unwrap();
        assert_eq!(homes.len(), 1, "{homes:?}");
        assert_eq!(homes[0].home, recent);
        let newest = aged(100)
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(homes[0].last_active, newest);
    }

    #[test]
    fn recent_session_homes_is_empty_without_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        std::fs::create_dir_all(&home).unwrap();
        assert!(CodexAdapter::new()
            .recent_session_homes(&home)
            .unwrap()
            .is_empty());
    }
}
