//! Claude session, sidecar, and memory discovery under one `CLAUDE_CONFIG_DIR`.

use super::materialization::encode_claude_project_path;
use crate::provider_capture::{
    canonical_existing_dir, collect_regular_tree_files, inspect_candidate_files,
    inspect_candidate_files_with_limit, modified_secs, normalized_relative, provider_walk_entry,
    session_home_is_project_root, MAX_METADATA_BYTES, MAX_METADATA_LINES,
};
use crate::providers::traits::{
    DiscoveredProviderState, DiscoveredSession, DiscoveredSidecar, DiscoveredStateFile,
    SessionDiscovery, SessionDiscoveryContext, SessionHomeSighting,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Bounds on one `recent_session_homes` pass.
const MAX_RECENT_BUCKETS: usize = 4_096;
const MAX_RECENT_BUCKET_FILES: usize = 20_000;
/// Transcripts opened per project directory, newest first, until one names
/// the folder it ran in.
const RECENT_HEAD_READS_PER_BUCKET: usize = 3;

/// The folders recent sessions ran in. Claude keeps one directory per
/// project under `projects/`, so each directory costs a stat per transcript
/// (for its newest date) and one bounded head read — never a scan of every
/// transcript. The head names where the session started, which is a hint
/// here; ownership stays with [`discover`].
pub(super) fn recent_session_homes(
    provider_home: &Path,
) -> Result<Vec<SessionHomeSighting>, String> {
    let canonical_home = canonical_existing_dir(provider_home, "Claude home")?;
    let Ok(buckets) = std::fs::read_dir(canonical_home.join("projects")) else {
        return Ok(Vec::new());
    };
    let mut homes: BTreeMap<PathBuf, u64> = BTreeMap::new();
    for bucket in buckets.take(MAX_RECENT_BUCKETS).flatten() {
        // `DirEntry` type and metadata never follow a link.
        if !bucket.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(bucket.path()) else {
            continue;
        };
        let mut transcripts: Vec<(u64, PathBuf)> = files
            .take(MAX_RECENT_BUCKET_FILES)
            .flatten()
            .filter(|file| {
                file.file_type().is_ok_and(|kind| kind.is_file())
                    && file.path().extension().and_then(|value| value.to_str()) == Some("jsonl")
            })
            .filter_map(|file| {
                let metadata = file.metadata().ok()?;
                Some((modified_secs(&metadata), file.path()))
            })
            .collect();
        transcripts.sort_by(|left, right| right.cmp(left));
        let Some(&(newest, _)) = transcripts.first() else {
            continue;
        };
        let home = transcripts
            .iter()
            .take(RECENT_HEAD_READS_PER_BUCKET)
            .find_map(|(_, path)| first_recorded_cwd(path));
        if let Some(home) = home {
            let last_active = homes.entry(home).or_insert(0);
            *last_active = (*last_active).max(newest);
        }
    }
    Ok(homes
        .into_iter()
        .map(|(home, last_active)| SessionHomeSighting { home, last_active })
        .collect())
}

/// The first absolute `cwd` a transcript records within the bounded head.
fn first_recorded_cwd(path: &Path) -> Option<PathBuf> {
    #[derive(serde::Deserialize)]
    struct Head {
        cwd: Option<String>,
    }
    crate::sync_metrics::record_file_head_read(path);
    let file = crate::jsonl_tail_io::open_no_follow(path).ok()?;
    let mut reader = BufReader::new(file.take(MAX_METADATA_BYTES as u64));
    for _ in 0..MAX_METADATA_LINES {
        let mut line = Vec::new();
        if reader.read_until(b'\n', &mut line).ok()? == 0 {
            return None;
        }
        let Ok(Head { cwd: Some(cwd) }) = serde_json::from_slice::<Head>(&line) else {
            continue;
        };
        let home = PathBuf::from(&cwd);
        if home.is_absolute() && cwd.len() <= 4096 && !cwd.chars().any(char::is_control) {
            return Some(home);
        }
    }
    None
}

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

/// The one project-owned session `is_requested` accepts. A transcript is
/// named by its session, so the walk opens nothing until the requested
/// session is found; its head then supplies the ownership evidence
/// `discover` would have gathered. Sidecars and memory are not visited.
pub(super) fn locate_session(
    context: &SessionDiscoveryContext<'_>,
    is_requested: &mut dyn FnMut(&str, &Path) -> Result<bool, String>,
    scan_metadata: impl Fn(&Path) -> Result<crate::providers::traits::SessionFileMetadata, String>,
) -> Result<Option<DiscoveredSession>, String> {
    let canonical_home = canonical_existing_dir(context.provider_home, "Claude home")?;
    let projects = canonical_home.join("projects");
    if !projects.exists() {
        return Ok(None);
    }
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
        let Some(session_id) = entry.path().file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !is_requested(session_id, entry.path())? {
            continue;
        }
        let meta = scan_metadata(entry.path())?;
        if meta.session_id != session_id {
            return Err("Claude transcript names a different session than its file".into());
        }
        // The same session can sit in more than one project directory; the
        // one that ran in this project is the one requested.
        if !session_home_is_project_root(
            context.project_root,
            &meta.authorized_home,
            &mut canonical_homes,
        ) {
            continue;
        }
        return Ok(Some(DiscoveredSession {
            session_id: meta.session_id.clone(),
            physical_path: entry.path().to_path_buf(),
            display_name: meta.session_id,
            blocked_reason: inspect_candidate_files_with_limit(
                &canonical_home,
                &[entry.path().to_path_buf()],
                crate::app_settings::conversation_file_limit_bytes(),
            )
            .err(),
        }));
    }
    Ok(None)
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

    /// Only the requested transcript is opened. A session named the same in
    /// a foreign project directory is passed over once its head names
    /// another folder, and a name no request matches is never opened.
    #[test]
    fn locate_session_opens_only_the_requested_transcript() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("claude");
        let project = temp.path().join("project");
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let home = home.canonicalize().unwrap();
        let project = project.canonicalize().unwrap();
        let bucket = home
            .join("projects")
            .join(encode_claude_project_path(project.to_str().unwrap()));
        let foreign = home
            .join("projects")
            .join(encode_claude_project_path(elsewhere.to_str().unwrap()));
        std::fs::create_dir_all(&bucket).unwrap();
        std::fs::create_dir_all(&foreign).unwrap();
        let write = |bucket: &Path, session_id: &str, cwd: &Path| {
            let record = serde_json::json!({
                "type": "user", "uuid": "u1", "parentUuid": null,
                "sessionId": session_id, "cwd": cwd,
                "message": {"role": "user", "content": "hello"}
            });
            let path = bucket.join(format!("{session_id}.jsonl"));
            std::fs::write(&path, format!("{record}\n")).unwrap();
            path
        };
        let wanted = write(&bucket, "session-wanted", &project);
        write(&foreign, "session-foreign", &elsewhere);
        write(&foreign, "session-twin", &elsewhere);
        let twin = write(&bucket, "session-twin", &project);

        let context = SessionDiscoveryContext {
            provider_home: &home,
            project_root: &project,
        };
        let adapter = crate::providers::claude::ClaudeAdapter::new();
        let scan = |path: &Path| crate::providers::SessionAdapter::scan_metadata(&adapter, path);
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
            )
            .unwrap();
            (
                found,
                metrics.snapshot().delta_since(&before).files_head_read,
            )
        };

        let mut offered = BTreeSet::new();
        let (found, reads) = locate("session-wanted", &mut offered);
        let found = found.unwrap();
        assert_eq!(found.physical_path, wanted);
        assert_eq!(found.display_name, "session-wanted");
        assert!(found.blocked_reason.is_none());
        assert_eq!(reads, 1);

        let (found, reads) = locate("session-foreign", &mut offered);
        assert!(found.is_none());
        assert_eq!(reads, 1);

        let (found, reads) = locate("session-twin", &mut offered);
        assert_eq!(found.unwrap().physical_path, twin);
        assert!(reads <= 2, "{reads}");

        let (found, reads) = locate("session-missing", &mut offered);
        assert!(found.is_none());
        assert_eq!(reads, 0);
        assert_eq!(
            offered,
            BTreeSet::from([
                "session-foreign".to_string(),
                "session-twin".to_string(),
                "session-wanted".to_string(),
            ])
        );
    }

    /// One project directory is one folder: dated by its newest transcript,
    /// named by the first `cwd` in that transcript's head. Nested transcripts
    /// (subagents) and linked directories are never read.
    #[test]
    fn recent_session_homes_reads_one_head_per_project_directory() {
        use std::time::{Duration, SystemTime, UNIX_EPOCH};
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".claude");
        let bucket = home.join("projects/-work-app");
        std::fs::create_dir_all(bucket.join("newest/subagents")).unwrap();
        let app = temp.path().join("work/app");
        // The clock is read once. Every file time and the expected time
        // come from this reading, however long the test takes.
        let now = SystemTime::now();
        let write = |name: &str, lines: &[serde_json::Value], age_secs: u64| {
            let path = bucket.join(name);
            let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
            std::fs::write(&path, body).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(now - Duration::from_secs(age_secs))
                .unwrap();
        };
        write(
            "older.jsonl",
            &[serde_json::json!({ "type": "user", "cwd": temp.path().join("work/elsewhere") })],
            5_000,
        );
        write(
            "newest.jsonl",
            &[
                serde_json::json!({ "type": "summary", "summary": "Earlier" }),
                serde_json::json!({ "type": "user", "cwd": "relative/ignored" }),
                serde_json::json!({ "type": "user", "cwd": app }),
            ],
            60,
        );
        std::fs::write(
            bucket.join("newest/subagents/agent-1.jsonl"),
            format!(
                "{}\n",
                serde_json::json!({ "cwd": temp.path().join("work/child") })
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            let elsewhere = temp.path().join("linked-bucket");
            std::fs::create_dir_all(&elsewhere).unwrap();
            std::fs::write(
                elsewhere.join("linked.jsonl"),
                format!(
                    "{}\n",
                    serde_json::json!({ "cwd": temp.path().join("work/linked") })
                ),
            )
            .unwrap();
            std::os::unix::fs::symlink(&elsewhere, home.join("projects/linked")).unwrap();
        }

        let homes = recent_session_homes(&home).unwrap();
        assert_eq!(homes.len(), 1, "{homes:?}");
        assert_eq!(homes[0].home, app);
        let newest = (now - Duration::from_secs(60))
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(homes[0].last_active, newest);
    }

    #[test]
    fn recent_session_homes_is_empty_without_projects() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".claude");
        std::fs::create_dir_all(&home).unwrap();
        assert!(recent_session_homes(&home).unwrap().is_empty());
    }

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
                    is_empty: false,
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
