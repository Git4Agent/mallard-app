//! Claude session, sidecar, and memory discovery under one `CLAUDE_CONFIG_DIR`.

use crate::provider_capture::{
    canonical_existing_dir, collect_regular_tree_files, inspect_candidate_files,
    inspect_candidate_files_with_limit, normalized_relative, owned_relative_cwd,
    provider_walk_entry, relative_cwd_key,
};
use crate::providers::traits::{
    DiscoveredProviderState, DiscoveredSession, DiscoveredSidecar, DiscoveredStateFile,
    SessionDiscovery, SessionDiscoveryContext,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use walkdir::WalkDir;

fn sidecar_session_id(
    sidecar_kind: &str,
    relative: &str,
    session_ids: &BTreeSet<String>,
) -> Result<Option<String>, String> {
    match sidecar_kind {
        "file-history" => {
            let Some((session_id, remainder)) = relative.split_once('/') else {
                return Ok(None);
            };
            if remainder.is_empty() || !session_ids.contains(session_id) {
                return Ok(None);
            }
            Ok(Some(session_id.to_string()))
        }
        "todos" => {
            if relative.contains('/') {
                return Ok(None);
            }
            let Some(stem) = relative.strip_suffix(".json") else {
                return Ok(None);
            };
            let matches = session_ids
                .iter()
                .filter(|session_id| {
                    stem.strip_prefix(session_id.as_str())
                        .and_then(|suffix| suffix.strip_prefix("-agent-"))
                        .is_some_and(|agent_id| !agent_id.is_empty())
                })
                .cloned()
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [] => Ok(None),
                [session_id] => Ok(Some(session_id.clone())),
                _ => Err("todo filename matches multiple proven session IDs".to_string()),
            }
        }
        _ => Err(format!(
            "unsupported Claude sidecar kind '{}'",
            sidecar_kind
        )),
    }
}

pub(super) fn discover(
    context: &SessionDiscoveryContext<'_>,
    scan_metadata: impl Fn(
        &std::path::Path,
    ) -> Result<crate::providers::traits::SessionFileMetadata, String>,
) -> Result<SessionDiscovery, String> {
    let canonical_home = canonical_existing_dir(context.provider_home, "Claude home")?;
    let mut discovery = SessionDiscovery::default();
    let projects = canonical_home.join("projects");
    if !projects.exists() {
        return Ok(discovery);
    }
    let mut session_ids = BTreeSet::new();
    let mut session_buckets = BTreeMap::<PathBuf, String>::new();
    for entry in WalkDir::new(&projects)
        .follow_links(false)
        .min_depth(2)
        .max_depth(3)
        .into_iter()
        .filter_entry(provider_walk_entry)
    {
        let entry = entry.map_err(|e| format!("walk Claude projects: {}", e))?;
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
        {
            continue;
        }
        let meta = match scan_metadata(entry.path()) {
            Ok(meta) => meta,
            Err(error) => {
                discovery.warnings.push(format!(
                    "ignored Claude session '{}': {}",
                    entry.path().display(),
                    error
                ));
                continue;
            }
        };
        let Some(relative_cwd) =
            owned_relative_cwd(context.project_root, &meta.cwd, context.excluded_roots)?
        else {
            continue;
        };
        session_ids.insert(meta.session_id.clone());
        if let Some(parent) = entry.path().parent() {
            session_buckets.insert(parent.to_path_buf(), relative_cwd.clone());
        }
        discovery.sessions.push(DiscoveredSession {
            session_id: meta.session_id.clone(),
            relative_cwd,
            physical_path: entry.path().to_path_buf(),
            display_name: meta.session_id,
            blocked_reason: inspect_candidate_files_with_limit(
                &canonical_home,
                &[entry.path().to_path_buf()],
                mallard_domain::MAX_CONVERSATION_FILE_BYTES,
            )
            .err(),
        });
    }

    // Sidecars are associated only through the provider's exact path grammar
    // and an already-proven session ID. Ambiguous todo names are rejected.
    for directory in ["file-history", "todos"] {
        let sidecar_root = canonical_home.join(directory);
        if !sidecar_root.exists() {
            continue;
        }
        for entry in WalkDir::new(&sidecar_root)
            .follow_links(false)
            .max_depth(4)
            .into_iter()
            .filter_entry(provider_walk_entry)
        {
            let entry = entry.map_err(|e| format!("walk Claude {}: {}", directory, e))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = normalized_relative(&sidecar_root, entry.path())?;
            let session_id = match sidecar_session_id(directory, &relative, &session_ids) {
                Ok(Some(session_id)) => session_id,
                Ok(None) => continue,
                Err(error) => {
                    discovery.warnings.push(format!(
                        "ignored Claude {} sidecar '{}': {}",
                        directory, relative, error
                    ));
                    continue;
                }
            };
            discovery.sidecars.push(DiscoveredSidecar {
                session_id,
                sidecar_kind: directory.to_string(),
                file_name: relative,
                physical_path: entry.path().to_path_buf(),
                blocked_reason: inspect_candidate_files_with_limit(
                    &canonical_home,
                    &[entry.path().to_path_buf()],
                    mallard_domain::MAX_CONVERSATION_FILE_BYTES,
                )
                .err(),
            });
        }
    }

    // Project memories live within a verified bucket; discovered only when a
    // session in that bucket proved project ownership.
    for (bucket, relative_cwd) in session_buckets {
        let memory = bucket.join("memory");
        if !memory.exists() {
            continue;
        }
        let files = collect_regular_tree_files(&canonical_home, memory.clone())?;
        if files.is_empty() {
            continue;
        }
        let relative_key = relative_cwd_key(&relative_cwd);
        let state_files = files
            .iter()
            .map(|path| {
                normalized_relative(&memory, path).map(|rel| DiscoveredStateFile {
                    physical_path: path.clone(),
                    logical_suffix: format!("memory/{}/{}", relative_key, rel),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        discovery.state_resources.push(DiscoveredProviderState {
            state_component: relative_cwd.clone(),
            resource_type: "memory".to_string(),
            display_name: "Claude project memory".to_string(),
            relative_cwd: Some(relative_cwd),
            merge_apply: true,
            files: state_files,
            blocked_reason: inspect_candidate_files(&canonical_home, &files).err(),
        });
    }
    Ok(discovery)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_history_uses_an_exact_first_path_component() {
        let ids = BTreeSet::from(["session".to_string(), "session-agent-child".to_string()]);

        assert_eq!(
            sidecar_session_id("file-history", "session-agent-child/file@v1", &ids).unwrap(),
            Some("session-agent-child".to_string())
        );
        assert_eq!(
            sidecar_session_id("file-history", "prefix-session/file@v1", &ids).unwrap(),
            None
        );
        assert_eq!(
            sidecar_session_id("file-history", "session-file@v1", &ids).unwrap(),
            None
        );
    }

    #[test]
    fn todo_filename_rejects_ambiguous_or_incidental_session_ids() {
        let ids = BTreeSet::from(["session".to_string(), "session-agent-child".to_string()]);

        assert_eq!(
            sidecar_session_id("todos", "session-agent-task.json", &ids).unwrap(),
            Some("session".to_string())
        );
        assert_eq!(
            sidecar_session_id("todos", "prefix-session-agent-task.json", &ids).unwrap(),
            None
        );
        assert!(
            sidecar_session_id("todos", "session-agent-child-agent-task.json", &ids)
                .unwrap_err()
                .contains("multiple proven session IDs")
        );
    }
}
