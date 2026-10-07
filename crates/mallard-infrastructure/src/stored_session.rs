//! A stored session's transcript, read for its page without a Pull.
//!
//! The bytes are prepared exactly as a Pull would install them
//! (`pull_transcript`): digests checked, the Codex standalone root rebuilt
//! with its display records, paths mapped back to this project's root, the
//! redaction stamp honoured. They are written once into a private temporary
//! provider home that lives as long as this value, so nothing of a stored
//! session is kept between commands and nothing is written under a real
//! provider home.

use crate::managed_cloud::views::safe_stem;
use crate::project_activity::ResolvedActivityProject;
use crate::providers::AgentProviderRegistry;
use crate::pull_transcript::{prepare_pull_transcript, PullTranscriptRequest};
use crate::session_detail::project_root_aliases;
use mallard_domain::{sha256_hex, ConversationThread, ProviderKey};
use std::fs;
use std::path::{Path, PathBuf};

/// One stored session, prepared and written into its own temporary home.
/// Dropping it removes the home.
pub struct StoredSessionTranscript {
    provider: ProviderKey,
    resource_id: String,
    provider_session_id: String,
    display_name: String,
    project_root: PathBuf,
    project_root_aliases: Vec<PathBuf>,
    wire_sha256: String,
    _home: tempfile::TempDir,
    canonical_home: PathBuf,
    transcript_path: PathBuf,
}

impl StoredSessionTranscript {
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }

    /// The stored row's id, `<provider>:conversation:cloudthread-…`.
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    pub fn provider_session_id(&self) -> &str {
        &self.provider_session_id
    }

    /// The manifest's display title, or the session id when it has none.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// This project's canonical root: checkpoints and path links read it.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn project_root_aliases(&self) -> &[PathBuf] {
        &self.project_root_aliases
    }

    /// SHA-256 of the stored wire object: what names a stored snapshot.
    pub fn wire_sha256(&self) -> &str {
        &self.wire_sha256
    }

    /// The private home, canonical, so child lookups and boundary text
    /// compare the spelling the transcript path carries.
    pub fn provider_home(&self) -> &Path {
        &self.canonical_home
    }

    pub fn transcript_path(&self) -> &Path {
        &self.transcript_path
    }
}

/// Prepare `wire_bytes`, the verified object the manifest names for
/// `thread`, as a Pull into `project` would, and write the result into a
/// private temporary home. `resource_id` is the stored row's id.
pub fn prepare_stored_session(
    registry: &AgentProviderRegistry,
    project: &ResolvedActivityProject,
    resource_id: &str,
    thread: &ConversationThread,
    wire_bytes: &[u8],
) -> Result<StoredSessionTranscript, String> {
    let adapter = registry.require(&thread.provider)?;
    let cloud_tail = thread
        .portable_tail
        .as_ref()
        .ok_or("the stored session has no accepted tail")?;
    // The session keeps its own id: nothing is installed, so no local id
    // can be occupied by it.
    let provider_session_id = thread
        .source_thread_id
        .clone()
        .unwrap_or_else(|| thread.thread_id.to_string());
    let prepared = prepare_pull_transcript(
        adapter,
        wire_bytes,
        &PullTranscriptRequest {
            cloud_thread_id: thread.thread_id.as_str(),
            cloud_tail,
            cloud_wire_redaction: thread.wire_redaction.as_ref(),
            cloud_wire_path_map: thread.wire_path_map.as_ref(),
            lineage_digest: Some(thread.lineage_digest.as_str()),
            semantic_digest: Some(thread.semantic_digest.as_str()),
            destination_provider_thread_id: &provider_session_id,
            project_root: &project.binding.project_root,
            canonical_project_root: &project.binding.canonical_project_root,
        },
    )?;
    let home = tempfile::tempdir().map_err(|error| format!("stored session workspace: {error}"))?;
    let canonical_home = fs::canonicalize(home.path())
        .map_err(|error| format!("stored session workspace: {error}"))?;
    let sessions = canonical_home.join("sessions");
    fs::create_dir_all(&sessions).map_err(|error| format!("stored session workspace: {error}"))?;
    let transcript_path = sessions.join(format!("{}.jsonl", safe_stem(&provider_session_id)));
    fs::write(&transcript_path, prepared.transcript.bytes().as_bytes())
        .map_err(|error| format!("stored session workspace: {error}"))?;
    let display_name = thread
        .display_title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| provider_session_id.clone());
    Ok(StoredSessionTranscript {
        provider: thread.provider.clone(),
        resource_id: resource_id.to_owned(),
        provider_session_id,
        display_name,
        project_root: project.project_root.clone(),
        project_root_aliases: project_root_aliases(project),
        wire_sha256: sha256_hex(wire_bytes),
        _home: home,
        canonical_home,
        transcript_path,
    })
}
