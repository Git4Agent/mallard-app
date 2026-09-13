//! Retained physical sources behind one logical timeline. Only the source
//! table is cached. Historical contents are assumed stable after indexing.

use crate::codex_history_graph::resolve_captured_codex_root;
use crate::codex_subagents::RolloutLookup;
use crate::jsonl_tail_io::{open_no_follow, read_snapshot};
use crate::payload::PayloadBytes;
use crate::providers::traits::{SessionSourceSpan, SessionTimelineIndex, SessionTimelineSource};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

fn relative_path(home: &Path, path: &Path) -> Result<String, String> {
    let canonical_home =
        fs::canonicalize(home).map_err(|_| "cannot resolve timeline provider home")?;
    let relative = path
        .strip_prefix(home)
        .or_else(|_| path.strip_prefix(&canonical_home))
        .map_err(|_| "timeline source escaped provider home")?;
    relative
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "timeline source path is not UTF-8".into())
}

fn source_path(home: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty() || relative.len() > 4096 {
        return Err("invalid timeline source path".into());
    }
    let home = fs::canonicalize(home).map_err(|_| "cannot resolve timeline provider home")?;
    let mut path = home.clone();
    for component in Path::new(relative).components() {
        let Component::Normal(part) = component else {
            return Err("invalid timeline source path".into());
        };
        path.push(part);
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| "timeline history source is missing")?;
        if metadata.file_type().is_symlink() {
            return Err("timeline source traverses a symbolic link".into());
        }
    }
    if !fs::canonicalize(&path)
        .map_err(|_| "cannot resolve timeline source")?
        .starts_with(home)
    {
        return Err("timeline source escaped provider home".into());
    }
    if !fs::symlink_metadata(&path)
        .map_err(|_| "cannot inspect timeline source")?
        .is_file()
    {
        return Err("timeline source is not a regular file".into());
    }
    Ok(path)
}

pub(super) fn capture(
    path: &Path,
    home: &Path,
    lookup: &RolloutLookup,
) -> Result<(Vec<SessionTimelineSource>, Vec<PayloadBytes>), String> {
    let relative = relative_path(home, path)?;
    let path = source_path(home, &relative)?;
    let (bytes, stamp) = read_snapshot(&path)?;
    // Ordinary records remain the timeline parser's responsibility (including
    // malformed-record warnings). Any declared base requires strict resolution.
    let has_base = bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|raw| serde_json::from_slice::<serde_json::Value>(raw).ok())
        .find(|value| crate::codex_rollout::structural_record_type(value) == Some("session_meta"))
        .is_some_and(|value| {
            value.get("payload").is_some_and(|payload| {
                ["history_base", "historyBase"]
                    .iter()
                    .any(|key| payload.get(key).is_some_and(|base| !base.is_null()))
            })
        });
    if !has_base {
        return Ok((
            vec![SessionTimelineSource {
                relative_path: relative,
                length: bytes.len() as u64,
            }],
            vec![bytes.into()],
        ));
    }
    let (resolved, mut snapshots) =
        resolve_captured_codex_root(&path, bytes.into(), stamp, &|id| {
            lookup
                .find_rollout(id)?
                .map(|path| source_path(home, &relative_path(home, &path)?))
                .transpose()
        })?;
    let mut sources = Vec::new();
    let mut contents = Vec::new();
    for segment in resolved.segments {
        sources.push(SessionTimelineSource {
            relative_path: relative_path(home, &segment.path)?,
            length: segment.consumed_byte_len,
        });
        contents.push(
            snapshots
                .remove(&segment.path)
                .ok_or("missing captured timeline source")?
                .0,
        );
    }
    Ok((sources, contents))
}

pub(super) struct TimelineSources {
    pub files: Vec<(File, u64)>,
}

impl TimelineSources {
    pub fn open(path: &Path, index: &SessionTimelineIndex, home: &Path) -> Result<Self, String> {
        if index.sources.is_empty()
            || index.sources.len() > crate::codex_rollout::MAX_CODEX_FORK_DEPTH + 1
        {
            return Err("invalid timeline source count".into());
        }
        if index.sources.last().map(|s| s.relative_path.as_str())
            != Some(relative_path(home, path)?.as_str())
        {
            return Err("timeline current source changed".into());
        }
        let mut total = 0u64;
        let mut files = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for source in &index.sources {
            if source.length > crate::app_settings::conversation_file_limit_bytes() {
                return Err("timeline source exceeds the transcript limit".into());
            }
            total = total
                .checked_add(source.length)
                .filter(|total| *total <= crate::provider_capture::MAX_CAPTURE_BYTES)
                .ok_or("timeline history exceeds the byte limit")?;
            if !seen.insert(&source.relative_path) {
                return Err("duplicate timeline source".into());
            }
            let file = open_no_follow(&source_path(home, &source.relative_path)?)?;
            let metadata = file
                .metadata()
                .map_err(|_| "cannot inspect timeline source")?;
            if !metadata.is_file() || metadata.len() < source.length {
                return Err(
                    "timeline source is not a regular file or is shorter than its cutoff".into(),
                );
            }
            files.push((file, source.length));
        }
        Ok(Self { files })
    }

    pub fn read_value(&mut self, span: SessionSourceSpan) -> Result<serde_json::Value, String> {
        if span.length == 0
            || span.length > (super::history::max_line_bytes() as u64).saturating_add(2)
        {
            return Err("timeline source span is outside the record limit".into());
        }
        let end = span
            .offset
            .checked_add(span.length)
            .ok_or("timeline source span overflowed")?;
        let mut start = 0;
        for (file, length) in &mut self.files {
            if span.offset >= start && end <= start + *length {
                file.seek(SeekFrom::Start(span.offset - start))
                    .map_err(|_| "cannot seek timeline source")?;
                let mut bytes = vec![0; span.length as usize];
                file.read_exact(&mut bytes)
                    .map_err(|_| "cannot read retained timeline record")?;
                return serde_json::from_slice(&bytes)
                    .map_err(|_| "timeline source record is malformed".into());
            }
            start += *length;
        }
        Err("timeline source span crosses or exceeds a retained segment".into())
    }
}
