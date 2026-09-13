//! Codex session discovery under one `CODEX_HOME`.

use super::filter_codex_session_index_by_ids;
use crate::provider_capture::{
    canonical_existing_dir, inspect_candidate_files_with_limit, provider_walk_entry,
    session_home_is_project_root,
};
use crate::providers::codex::history::parse_session_index_titles;
use crate::providers::traits::{DiscoveredSession, SessionDiscovery, SessionDiscoveryContext};
use std::collections::{BTreeMap, BTreeSet};
use walkdir::WalkDir;

const MAX_PROFILE_SESSION_FILES: usize = 20_000;
const MAX_PROFILE_SESSION_ENTRIES: usize = 80_000;

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
