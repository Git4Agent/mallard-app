//! Claude materialization target planning: where reviewed pull actions may
//! write inside a Claude home. The adapter proposes targets; the core
//! performs the final canonical-boundary, no-follow, collision, size, and
//! digest checks.

use crate::providers::traits::{MaterializationTargetRequest, PlannedProviderTarget};
use mallard_domain::ConversationFileRole;
use std::path::{Path, PathBuf};

/// Claude buckets sessions per working directory using a lossy
/// alphanumeric-or-dash encoding of the absolute cwd.
pub fn encode_claude_project_path(path: &str) -> String {
    path.encode_utf16()
        .map(|unit| match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() => byte as char,
            _ => '-',
        })
        .collect()
}

fn validated_session_file_name(session_id: &str) -> Result<String, String> {
    if session_id.is_empty()
        || session_id.len() > 128
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("Claude session id is not a safe file-name component".to_string());
    }
    Ok(format!("{}.jsonl", session_id))
}

pub(super) fn materialization_targets(
    request: &MaterializationTargetRequest<'_>,
) -> Result<Vec<PlannedProviderTarget>, String> {
    let mut targets = Vec::new();
    let transcript = match request.existing_transcript_path {
        Some(existing) => existing.to_path_buf(),
        None => {
            let bucket = encode_claude_project_path(request.project_root);
            if bucket.is_empty() {
                return Err("Claude project bucket encoding is empty".to_string());
            }
            request
                .provider_home
                .join("projects")
                .join(bucket)
                .join(validated_session_file_name(request.provider_session_id)?)
        }
    };
    targets.push(PlannedProviderTarget {
        target_path: transcript,
        role: ConversationFileRole::Transcript,
    });
    for (kind, relative) in request.sidecars {
        if !matches!(kind.as_str(), "todos" | "file-history") {
            return Err(format!("unsupported Claude sidecar kind '{}'", kind));
        }
        let renamed = match request.source_provider_session_id {
            Some(source) if source != request.provider_session_id => {
                relative.replace(source, request.provider_session_id)
            }
            _ => relative.clone(),
        };
        let mut target = request.provider_home.join(kind);
        for component in renamed.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return Err(format!("unsafe Claude sidecar path '{}'", renamed));
            }
            target.push(component);
        }
        targets.push(PlannedProviderTarget {
            target_path: target,
            role: ConversationFileRole::Sidecar {
                sidecar_kind: kind.clone(),
            },
        });
    }
    Ok(targets)
}

/// Map one provider-state logical suffix (`memory/<rel>`) to its machine
/// path in the project root's canonical bucket under the Claude home.
pub(super) fn state_target(
    provider_home: &Path,
    project_root: &str,
    logical_suffix: &str,
) -> Result<PathBuf, String> {
    let Some(remainder) = logical_suffix.strip_prefix("memory/") else {
        return Err(format!(
            "Claude state path '{}' is not project memory",
            logical_suffix
        ));
    };
    let bucket = encode_claude_project_path(project_root);
    let mut target = provider_home.join("projects").join(bucket).join("memory");
    for component in remainder.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(format!("unsafe Claude memory path '{}'", logical_suffix));
        }
        target.push(component);
    }
    Ok(target)
}
