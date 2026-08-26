//! Codex session discovery under one `CODEX_HOME`.

use crate::provider_capture::{
    canonical_existing_dir, filter_codex_session_index_by_ids, inspect_candidate_files_with_limit,
    provider_walk_entry, session_home_is_project_root,
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
            discovery
                .profile_session_ids
                .insert(meta.session_id.clone());
            if !session_home_is_project_root(context.project_root, &meta.cwd) {
                continue;
            }
            if meta.is_internal_subagent {
                // TODO(sync-subagent-task): Parent/child rollout graphs need
                // atomic selection and complete parent-ID rewriting before
                // they can be portable. See docs/TODO.md.
                discovery
                    .excluded_subagent_session_ids
                    .insert(meta.session_id);
                continue;
            }
            // Codex can briefly leave the same transcript in both `sessions`
            // and `archived_sessions` while an archive operation settles. A
            // session ID is the stable resource identity, so keep the active
            // `sessions` copy discovered first and ignore later copies.
            if directory == "archived_sessions" {
                if let Some(existing) = by_session.get(&meta.session_id) {
                    let existing_path = discovery.sessions[*existing].physical_path.clone();
                    discovery.warnings.push(format!(
                        "ignored duplicate Codex session '{}' at '{}'; using '{}'",
                        meta.session_id,
                        entry.path().display(),
                        existing_path.display(),
                    ));
                    continue;
                }
            }
            selected_thread_ids.insert(meta.session_id.clone());
            by_session.insert(meta.session_id.clone(), discovery.sessions.len());
            discovery.sessions.push(DiscoveredSession {
                session_id: meta.session_id.clone(),
                physical_path: entry.path().to_path_buf(),
                display_name: meta.session_id,
                blocked_reason: inspect_candidate_files_with_limit(
                    &canonical_home,
                    &[entry.path().to_path_buf()],
                    crate::app_settings::conversation_file_limit_bytes(),
                )
                .err(),
            });
        }
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
