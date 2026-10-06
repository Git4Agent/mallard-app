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

/// A valid standalone header can represent an empty session. The metadata
/// scanner also proves it is the file's only complete record before hiding it.
pub(super) fn is_standalone_session_header(line: &[u8], session_id: &str) -> bool {
    let Ok(record) = serde_json::from_slice::<serde_json::Value>(line) else {
        return false;
    };
    if record.get("type").and_then(serde_json::Value::as_str) != Some("session_meta") {
        return false;
    }
    let mut structure = crate::codex_rollout::CodexStructureAccumulator::new(Some(session_id));
    if structure.feed(&record).is_err() {
        return false;
    }
    let Ok(structure) = structure.finalize() else {
        return false;
    };
    let meta = structure.current;
    meta.history_base.is_none()
        && meta.forked_from_id.is_none()
        && ["parent_thread_id", "parentThreadId"].iter().all(|key| {
            record["payload"]
                .get(*key)
                .is_none_or(serde_json::Value::is_null)
        })
        && match meta.history_mode.as_deref() {
            None | Some("legacy") => true,
            Some("paginated") => {
                record.get("ordinal").and_then(serde_json::Value::as_u64) == Some(0)
            }
            _ => false,
        }
}

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
        if metadata.is_internal_subagent || metadata.is_empty {
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
        let blocked_reason = selection_error.or_else(|| {
            inspect_candidate_files_with_limit(
                &canonical_home,
                std::slice::from_ref(&physical_path),
                crate::app_settings::conversation_file_limit_bytes(),
            )
            .err()
        });
        if blocked_reason.is_none() && meta.is_empty {
            continue;
        }
        selected_thread_ids.insert(session_id.clone());
        by_session.insert(session_id.clone(), discovery.sessions.len());
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

/// The one project-owned session `is_requested` accepts. Native file names
/// carry each rollout's thread, so the walk opens nothing until the
/// requested thread is found; its current rollout's head then supplies the
/// identity and ownership evidence `discover` would have gathered. The
/// session index is never read: the name is the thread id.
pub(super) fn locate_session(
    context: &SessionDiscoveryContext<'_>,
    is_requested: &mut dyn FnMut(&str, &Path) -> Result<bool, String>,
    scan_metadata: impl Fn(&Path) -> Result<crate::providers::traits::SessionFileMetadata, String>,
) -> Result<Option<DiscoveredSession>, String> {
    let canonical_home = canonical_existing_dir(context.provider_home, "Codex home")?;
    let lookup = crate::codex_subagents::RolloutLookup::new(&canonical_home);
    let mut canonical_homes = BTreeMap::new();
    for (session_id, paths) in lookup.rollouts_by_thread()? {
        let (physical_path, selection_error) = match lookup.select_current(&session_id, &paths) {
            Ok(Some(path)) => (path, None),
            Ok(None) => continue,
            Err(error) => (paths[0].clone(), Some(error)),
        };
        if !is_requested(&session_id, &physical_path)? {
            continue;
        }
        let meta = scan_metadata(&physical_path)?;
        if meta.session_id != session_id {
            return Err("Codex rollout header disagrees with its filename identity".into());
        }
        if meta.is_internal_subagent
            || !session_home_is_project_root(
                context.project_root,
                &meta.authorized_home,
                &mut canonical_homes,
            )
        {
            return Ok(None);
        }
        let blocked_reason = selection_error.or_else(|| {
            inspect_candidate_files_with_limit(
                &canonical_home,
                std::slice::from_ref(&physical_path),
                crate::app_settings::conversation_file_limit_bytes(),
            )
            .err()
        });
        if blocked_reason.is_none() && meta.is_empty {
            return Ok(None);
        }
        return Ok(Some(DiscoveredSession {
            display_name: session_id.clone(),
            session_id,
            physical_path,
            blocked_reason,
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::codex::CodexAdapter;
    use crate::providers::traits::SessionAdapter;
    use std::time::{Duration, SystemTime};

    /// Only the requested thread's current rollout is opened. Other threads,
    /// the superseded copy, and the session index stay closed; the newest
    /// native name wins over modification time; a foreign folder, a
    /// subagent, and a thread no name claims yield nothing; a name that
    /// disagrees with its header is refused.
    #[test]
    fn locate_session_opens_only_the_requested_rollout() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let project = temp.path().join("project");
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(home.join("sessions/2026/09/12")).unwrap();
        std::fs::create_dir_all(home.join("archived_sessions")).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let home = home.canonicalize().unwrap();
        let project = project.canonicalize().unwrap();
        let day = home.join("sessions/2026/09/12");
        let write = |path: PathBuf, id: &str, cwd: &Path, subagent: bool| {
            let mut payload = serde_json::json!({ "id": id, "cwd": cwd });
            if subagent {
                payload["thread_source"] = serde_json::json!("subagent");
            }
            let header = serde_json::json!({ "type": "session_meta", "payload": payload });
            let message = serde_json::json!({
                "type": "event_msg", "payload": {"type": "user_message", "message": "Hello"}
            });
            std::fs::write(&path, format!("{header}\n{message}\n")).unwrap();
            path
        };
        let wanted = "0193b6c1-5352-7b12-9c6e-000000000001";
        let foreign = "0193b6c1-5352-7b12-9c6e-000000000002";
        let child = "0193b6c1-5352-7b12-9c6e-000000000003";
        let renamed = "0193b6c1-5352-7b12-9c6e-000000000004";
        let current = write(
            home.join(format!(
                "archived_sessions/rollout-2026-09-13T00-19-15-{wanted}_rollout-1.jsonl"
            )),
            wanted,
            &project,
            false,
        );
        // Written last: modification time would pick the superseded copy.
        write(
            day.join(format!("rollout-2026-09-12T01-03-07-{wanted}.jsonl")),
            wanted,
            &project,
            false,
        );
        write(
            day.join(format!("rollout-2026-09-12T01-03-08-{foreign}.jsonl")),
            foreign,
            &elsewhere,
            false,
        );
        write(
            day.join(format!("rollout-2026-09-12T01-03-09-{child}.jsonl")),
            child,
            &project,
            true,
        );
        write(
            day.join(format!("rollout-2026-09-12T01-03-10-{renamed}.jsonl")),
            foreign,
            &project,
            false,
        );
        std::fs::write(
            home.join("session_index.jsonl"),
            format!(
                "{}\n",
                serde_json::json!({"id": wanted, "thread_name": "Named in the index"})
            ),
        )
        .unwrap();

        let context = SessionDiscoveryContext {
            provider_home: &home,
            project_root: &project,
        };
        let adapter = CodexAdapter::new();
        let scan = |path: &Path| adapter.scan_metadata(path);
        let metrics = crate::sync_metrics::register_root(&home);
        let locate = |session_id: &str, offered: &mut BTreeSet<String>| {
            let before = metrics.snapshot();
            let found = locate_session(
                &context,
                &mut |candidate, _| {
                    offered.insert(candidate.to_string());
                    Ok(candidate == session_id)
                },
                scan,
            );
            (
                found,
                metrics.snapshot().delta_since(&before).files_head_read,
            )
        };

        let mut offered = BTreeSet::new();
        let (found, reads) = locate(wanted, &mut offered);
        let found = found.unwrap().unwrap();
        assert_eq!(found.physical_path, current);
        assert_eq!(found.display_name, wanted, "the index is not read");
        assert!(found.blocked_reason.is_none());
        assert_eq!(reads, 1);

        let (found, reads) = locate(foreign, &mut offered);
        assert!(found.unwrap().is_none());
        assert_eq!(reads, 1);

        let (found, reads) = locate(child, &mut offered);
        assert!(found.unwrap().is_none());
        assert_eq!(reads, 1);

        let (found, reads) = locate(renamed, &mut offered);
        assert_eq!(
            found.unwrap_err(),
            "Codex rollout header disagrees with its filename identity"
        );
        assert_eq!(reads, 1);

        let (found, reads) = locate("0193b6c1-5352-7b12-9c6e-000000000009", &mut offered);
        assert!(found.unwrap().is_none());
        assert_eq!(reads, 0);
        assert_eq!(
            offered,
            BTreeSet::from([wanted, foreign, child, renamed].map(String::from))
        );

        // The inventory still names the thread by its index title.
        let listed = adapter.discover(&context).unwrap();
        let listed = listed
            .sessions
            .iter()
            .find(|session| session.session_id == wanted)
            .unwrap();
        assert_eq!(listed.display_name, "Named in the index");
    }

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
        let message = serde_json::json!({
            "type": "event_msg", "payload": {"type": "user_message", "message": "Hello"}
        });
        std::fs::write(&path, format!("{record}\n{message}\n")).unwrap();
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

    #[test]
    fn metadata_only_sessions_are_ignored_until_content_is_appended() {
        use std::io::Write;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let project = temp.path().join("project");
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let home = home.canonicalize().unwrap();
        let project = project.canonicalize().unwrap();
        let path = home.join("sessions/rollout-empty.jsonl");
        let header = serde_json::json!({
            "type": "session_meta", "ordinal": 0,
            "payload": {"id": "empty", "cwd": project, "history_mode": "paginated"}
        });
        std::fs::write(&path, format!("{header}\n")).unwrap();
        let adapter = CodexAdapter::new();
        let context = SessionDiscoveryContext {
            provider_home: &home,
            project_root: &project,
        };
        let metrics = crate::sync_metrics::register_root(&path);
        assert!(adapter.discover(&context).unwrap().sessions.is_empty());
        assert!(adapter.recent_session_homes(&home).unwrap().is_empty());
        let before = metrics.snapshot();
        let empty = adapter.discover(&context).unwrap();
        assert!(empty.sessions.is_empty());
        assert!(empty.warnings.is_empty());
        assert_eq!(metrics.snapshot().delta_since(&before).files_head_read, 0);
        assert_eq!(metrics.snapshot().delta_since(&before).append_bytes_read, 0);

        let message = serde_json::json!({
            "type": "event_msg", "ordinal": 1,
            "payload": {"type": "user_message", "message": "Hello"}
        });
        let mut writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(writer, "{message}").unwrap();
        writer.flush().unwrap();
        let discovered = adapter.discover(&context).unwrap();
        assert_eq!(discovered.sessions.len(), 1);
        assert_eq!(discovered.sessions[0].session_id, "empty");
        assert!(discovered.sessions[0].blocked_reason.is_none());
        assert_eq!(adapter.recent_session_homes(&home).unwrap().len(), 1);
        drop(writer);
    }

    #[test]
    fn empty_filter_preserves_inherited_and_unproven_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollout-session.jsonl");
        let header = serde_json::json!({
            "type": "session_meta", "ordinal": 0,
            "payload": {"id": "session", "cwd": temp.path(), "history_mode": "paginated"}
        });
        for (name, field, value) in [
            (
                "history",
                "history_base",
                serde_json::json!({
                    "thread_id": "base", "end_ordinal_exclusive": 2, "end_byte_offset": 100
                }),
            ),
            ("fork", "forked_from_id", serde_json::json!("base")),
            ("parent", "parent_thread_id", serde_json::json!("base")),
            (
                "malformed history",
                "history_base",
                serde_json::json!("broken"),
            ),
        ] {
            let mut record = header.clone();
            record["payload"][field] = value;
            std::fs::write(&path, format!("{record}\n")).unwrap();
            assert!(
                !CodexAdapter::new().scan_metadata(&path).unwrap().is_empty,
                "{name}"
            );
        }
        let mut bad_ordinal = header.clone();
        bad_ordinal["ordinal"] = serde_json::json!(4);
        for bytes in [
            format!("{header}"),
            format!("{header}\n{{broken\n"),
            format!("{header}\n{{\"type\":\"event_msg\"}}\n"),
            format!("{bad_ordinal}\n"),
            format!("{}\n", serde_json::to_string_pretty(&header).unwrap()),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(!CodexAdapter::new()
                .scan_metadata(&path)
                .is_ok_and(|metadata| metadata.is_empty));
        }
    }
}
