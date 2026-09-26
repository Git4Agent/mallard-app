//! Claude session, sidecar, and memory discovery under one `CLAUDE_CONFIG_DIR`.

use super::materialization::encode_claude_project_path;
use crate::provider_capture::{
    canonical_existing_dir, collect_regular_tree_files, inspect_candidate_files,
    inspect_candidate_files_with_limit, normalized_relative, provider_walk_entry,
    session_home_is_project_root,
};
use crate::providers::traits::{
    DiscoveredProviderState, DiscoveredSession, DiscoveredSidecar, DiscoveredStateFile,
    SessionDiscovery, SessionDiscoveryContext,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Whether a regular file shares its data with another name. Claude writes
/// shared file-history snapshots as hard links on every platform.
fn is_hard_linked_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .ok()
        .and_then(|meta| crate::persistence::hard_link_count(&meta, path))
        .is_some_and(|links| links > 1)
}

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
    let mut root_buckets = BTreeSet::<PathBuf>::new();
    let mut canonical_homes = BTreeMap::new();
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
        if !session_home_is_project_root(
            context.project_root,
            &meta.authorized_home,
            &mut canonical_homes,
        ) {
            continue;
        }
        session_ids.insert(meta.session_id.clone());
        // The root bucket is the munged directory Claude itself uses for
        // this project: its name must match the munge of the session's own
        // recorded home. A foreign bucket holding a resumed-into-root
        // session fails this self-consistency check, and its memory stays
        // with its own directory-project.
        if let (Some(parent), Some(home_text)) =
            (entry.path().parent(), meta.authorized_home.to_str())
        {
            if parent.file_name().and_then(|name| name.to_str())
                == Some(encode_claude_project_path(home_text).as_str())
            {
                root_buckets.insert(parent.to_path_buf());
            }
        }
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
            // Shared snapshots are excluded; other failures still go
            // through the normal sidecar validation below.
            if directory == "file-history" && is_hard_linked_file(entry.path()) {
                continue;
            }
            discovery.sidecars.push(DiscoveredSidecar {
                session_id,
                sidecar_kind: directory.to_string(),
                file_name: relative,
                physical_path: entry.path().to_path_buf(),
                blocked_reason: inspect_candidate_files_with_limit(
                    &canonical_home,
                    &[entry.path().to_path_buf()],
                    crate::app_settings::conversation_file_limit_bytes(),
                )
                .err(),
            });
        }
    }

    // Project memory lives in the canonical root bucket only, and only when
    // an owned session proved that bucket (the munge is lossy). Distinct
    // spellings of the same root can each carry a self-consistent bucket;
    // one memory resource is emitted from the first, the rest are warned.
    let mut root_buckets = root_buckets.into_iter();
    if let Some(bucket) = root_buckets.next() {
        for extra in root_buckets {
            discovery.warnings.push(format!(
                "ignored Claude memory in '{}'; using '{}'",
                extra.display(),
                bucket.display()
            ));
        }
        let memory = bucket.join("memory");
        if memory.exists() {
            let files = collect_regular_tree_files(&canonical_home, memory.clone())?;
            if !files.is_empty() {
                let state_files = files
                    .iter()
                    .map(|path| {
                        normalized_relative(&memory, path).map(|rel| DiscoveredStateFile {
                            physical_path: path.clone(),
                            logical_suffix: format!("memory/{}", rel),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                discovery.state_resources.push(DiscoveredProviderState {
                    state_component: "root".to_string(),
                    resource_type: "memory".to_string(),
                    display_name: "Claude project memory".to_string(),
                    merge_apply: true,
                    files: state_files,
                    blocked_reason: inspect_candidate_files(&canonical_home, &files).err(),
                });
            }
        }
    }
    Ok(discovery)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn hard_linked_file_history_is_excluded_without_blocking_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("claude");
        let project = temp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let project = std::fs::canonicalize(project).unwrap();
        let bucket = home
            .join("projects")
            .join(encode_claude_project_path(project.to_str().unwrap()));
        std::fs::create_dir_all(&bucket).unwrap();
        for session_id in ["session-a", "session-b"] {
            std::fs::write(bucket.join(format!("{session_id}.jsonl")), b"{}\n").unwrap();
            std::fs::create_dir_all(home.join("file-history").join(session_id)).unwrap();
        }
        let shared = home.join("file-history/session-a/shared@v1");
        std::fs::write(&shared, b"shared snapshot").unwrap();
        std::fs::hard_link(&shared, home.join("file-history/session-b/shared@v1")).unwrap();
        let ordinary = home.join("file-history/session-a/ordinary@v1");
        std::fs::write(&ordinary, b"ordinary snapshot").unwrap();

        let discovery = discover(
            &SessionDiscoveryContext {
                provider_home: &home,
                project_root: &project,
            },
            |path| {
                Ok(crate::providers::traits::SessionFileMetadata {
                    session_id: path.file_stem().unwrap().to_str().unwrap().to_string(),
                    authorized_home: project.clone(),
                    is_internal_subagent: false,
                })
            },
        )
        .unwrap();

        assert_eq!(discovery.sessions.len(), 2);
        assert!(discovery
            .sessions
            .iter()
            .all(|s| s.blocked_reason.is_none()));
        assert!(discovery.warnings.is_empty());
        assert_eq!(discovery.sidecars.len(), 1);
        let sidecar = &discovery.sidecars[0];
        assert_eq!(sidecar.session_id, "session-a");
        assert_eq!(sidecar.sidecar_kind, "file-history");
        assert_eq!(
            sidecar.physical_path,
            std::fs::canonicalize(ordinary).unwrap()
        );
        assert!(sidecar.blocked_reason.is_none());
    }

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
