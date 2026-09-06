//! Best-effort provider-file materialization for conversation actions.
//!
//! New destinations use no-clobber publication. Existing transcripts use one
//! source check before installation; later transcript updates can be replaced.
//! Shared provider files retain digest checks through commit. A multi-file
//! action is not replayed, rolled back, or resumed after failure.

use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::payload::PayloadBytes;
pub use crate::persistence::ConversationPublicationState;
use crate::persistence::{LocalStateRepository, LockedConversationPull};
use crate::verified_transcript::VerifiedTranscript;
use mallard_domain::{
    conversation_resource_id, hex_lower, sha256_hex, ActionId, ActionStatus, ApplyReceipt,
    ConversationApplyKind, ConversationApplyOperation, ConversationApplyReceipt,
    ConversationApplyTarget, ConversationBindings, ConversationFileRole,
    ConversationMaterializationState, ConversationOperationId, ConversationPlanDirection,
    ConversationSourceDisposition, HostIdentity, MachineProjectState, MaterializationId,
    MaterializationRecord, MaterializationStatus, Materializations, RestoreActionType,
    TargetExpectation, LOCAL_SCHEMA, MAX_CONVERSATION_FILES,
};

const MAX_PROVIDER_TARGET_BYTES: u64 = 512 * 1024 * 1024;

struct StageCleanup(Vec<PathBuf>);

impl Drop for StageCleanup {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationTargetPayload {
    pub target_path: PathBuf,
    pub role: ConversationFileRole,
    pub expected: TargetExpectation,
    /// The provider index row ids this payload rewrites; non-empty exactly
    /// for `ProviderIndexFragment` payloads.
    pub index_rows: Vec<String>,
    pub bytes: PayloadBytes,
}

#[derive(Clone, Debug)]
pub struct ConversationMaterializationRequest {
    pub operation: ConversationApplyOperation,
    pub targets: Vec<ConversationTargetPayload>,
    pub transcript: VerifiedTranscript,
    /// Bytes returned by the source check, required only when the transcript
    /// is replaced in place. Kept in memory for its backup, never serialized.
    pub checked_transcript: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationCloudPublication {
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
}

/// Publish one Push apply operation and finalize its local receipt.
#[cfg(any(test, feature = "test-support"))]
pub fn apply_conversation_cloud_publication(
    repository: &LocalStateRepository,
    operation: ConversationApplyOperation,
    state: ConversationPublicationState,
    publish: impl FnOnce() -> Result<ConversationCloudPublication, String>,
) -> Result<ConversationApplyReceipt, String> {
    let _host_guard = repository.acquire_host_operation()?;
    let receipts =
        apply_conversation_cloud_publications_locked(repository, vec![operation], state, publish)?;
    receipts
        .into_iter()
        .next()
        .ok_or_else(|| "cloud publication produced no receipt".to_string())
}

/// Publish a reviewed set of Push conversation actions while the command
/// orchestrator holds the host-operation guard across generic and
/// conversation publication. Candidate objects, the immutable manifest, and
/// the candidate commit are already uploaded (orphan-safe) by the caller;
/// this function advances the head once by CAS and installs the local
/// after-states during the same attempt.
pub fn apply_conversation_cloud_publications_locked(
    repository: &LocalStateRepository,
    operations: Vec<ConversationApplyOperation>,
    mut state: ConversationPublicationState,
    publish: impl FnOnce() -> Result<ConversationCloudPublication, String>,
) -> Result<Vec<ConversationApplyReceipt>, String> {
    if operations.is_empty() {
        return Err("cloud publication requires at least one conversation action".to_string());
    }
    let mut operation_ids = std::collections::BTreeSet::new();
    let mut result_threads = std::collections::BTreeSet::new();
    let first = &operations[0];
    let current = repository.load_project_activity_state_snapshot()?;
    state.validate(&current)?;
    for operation in &operations {
        if !matches!(
            operation.operation,
            ConversationApplyKind::CloudPublication(_)
        ) || !operation.targets.is_empty()
        {
            return Err("cloud publication requires Push apply operations".to_string());
        }
        operation.validate()?;
        validate_operation_scope_from_state(
            &current.host_identity,
            &current.bindings,
            &current.conversation_bindings,
            operation,
        )?;
        if operation.plan_id != first.plan_id
            || operation.cloud != first.cloud
            || operation.host_id != first.host_id
            || operation.replica_id != first.replica_id
            || operation.local_project_id != first.local_project_id
        {
            return Err("conversation publication batch has inconsistent plan pins".to_string());
        }
        if !operation_ids.insert(operation.operation_id.clone())
            || !result_threads.insert(operation.thread_id.clone())
        {
            return Err(
                "conversation publication batch repeats an operation or result".to_string(),
            );
        }
    }

    drop(current);
    let publication = publish()?;
    let mut receipts = Vec::with_capacity(operations.len());
    let mut records = Vec::with_capacity(operations.len());
    for operation in &operations {
        // Only this batch's project and thread checkpoints accept the new
        // head. Other checkpoints retain their original publication pins.
        if let Some(accepted) = state
            .sync_config_after
            .links
            .iter_mut()
            .find(|link| {
                link.storage_id == operation.cloud.storage_id
                    && link.local_project_id == operation.local_project_id
            })
            .and_then(|link| {
                link.conversation_checkpoints
                    .get_mut(&mallard_domain::SyncScopeKey::Provider(
                        operation.provider.clone(),
                    ))
            })
            .and_then(|threads| threads.get_mut(&operation.thread_id))
            .and_then(|checkpoint| checkpoint.accepted.as_mut())
        {
            accepted.accepted_at_commit_id = publication.commit_id.clone();
            accepted.accepted_at_manifest_sha256 = publication.manifest_sha256.clone();
        }
        let (receipt, record) = prepare_receipt(
            operation,
            publication.revision,
            &publication.commit_id,
            &publication.manifest_sha256,
        )?;
        receipts.push(receipt);
        records.push(record);
    }
    repository.commit_conversation_publication(&state, |materializations| {
        for record in records {
            insert_receipt_record(materializations, record)?;
        }
        Ok(())
    })?;
    Ok(receipts)
}

/// Apply one reviewed Pull materialization. Existing transcripts have already
/// passed their one source check. Other targets and local scope revisions are
/// checked before any target stage is written.
#[cfg(any(test, feature = "test-support"))]
pub fn apply_conversation_materialization(
    repository: &LocalStateRepository,
    request: ConversationMaterializationRequest,
) -> Result<ConversationApplyReceipt, String> {
    let _host_guard = repository.acquire_host_operation()?;
    let mut state = repository.lock_conversation_pull()?;
    apply_conversation_materialization_locked(&mut state, request)
}

/// Apply one reviewed materializing Pull, including keep-both, while the
/// command orchestrator holds the host-operation and persistence guards. The
/// operation carries after-states and checks the batch's current revisions.
/// The source check authorizes an in-place transcript replacement; other
/// targets retain their digest guards.
pub fn apply_conversation_materialization_locked(
    state: &mut LockedConversationPull<'_>,
    mut request: ConversationMaterializationRequest,
) -> Result<ConversationApplyReceipt, String> {
    let current = state.current()?;
    request.operation.validate()?;
    let ConversationApplyKind::ProviderMaterialization(payload) = &request.operation.operation
    else {
        return Err("conversation materialization requires a Pull apply operation".to_string());
    };
    if payload.expected_config_revision != current.config.revision {
        return Err("sync config changed during apply".to_string());
    }
    validate_operation_scope_from_state(
        &current.host_identity,
        &current.bindings,
        &current.conversation_bindings,
        &request.operation,
    )?;
    verify_transcript_request(&current.bindings, &request)?;
    ensure_provider_target_parents(&current.bindings, &request.operation, &request.targets)?;
    let repository = state.repository();
    let checked_transcript = request.checked_transcript.take();
    let (operation, payloads) = prepare_operation(repository, request)?;
    validate_target_expectations(&operation.targets, checked_transcript.as_deref())?;
    let _stage_cleanup = StageCleanup(
        operation
            .targets
            .iter()
            .filter_map(|target| target.staged_path.as_ref().map(PathBuf::from))
            .collect(),
    );

    for (target, bytes) in operation.targets.iter().zip(&payloads) {
        let staged = Path::new(
            target
                .staged_path
                .as_deref()
                .ok_or_else(|| "conversation target lacks a staging path".to_string())?,
        );
        ensure_canonical_parent(staged)?;
        write_new_file(staged, bytes)?;
        verify_provider_payload(&operation, target, staged)?;
        if let (TargetExpectation::Sha256(expected), Some(backup)) =
            (&target.expected, target.backup_path.as_deref())
        {
            let backup = Path::new(backup);
            create_private_directory_all(
                repository.root(),
                backup
                    .parent()
                    .ok_or_else(|| "conversation backup has no parent".to_string())?,
            )?;
            if read_regular_file_digest(backup)?.is_none() {
                if replaces_transcript(target) {
                    write_new_file(
                        backup,
                        checked_transcript
                            .as_deref()
                            .expect("replacement requires checked source bytes"),
                    )?;
                } else {
                    copy_new_file(Path::new(&target.target_path), backup)?;
                }
            }
            if read_regular_file_digest(backup)?.as_deref() != Some(expected) {
                return Err("conversation backup digest mismatch".to_string());
            }
        }
    }

    for target in &operation.targets {
        commit_one_target(target)?;
    }

    let (receipt, record) = prepare_receipt(
        &operation,
        operation.cloud.revision,
        &operation.cloud.commit_id,
        &operation.cloud.manifest_sha256,
    )?;
    let ConversationApplyKind::ProviderMaterialization(payload) = operation.operation else {
        unreachable!("materialization operation was checked above");
    };
    state.commit(
        payload.sync_config_after,
        payload.bindings_after,
        |records| insert_receipt_record(records, record),
    )?;
    Ok(receipt)
}

fn prepare_operation(
    repository: &LocalStateRepository,
    request: ConversationMaterializationRequest,
) -> Result<(ConversationApplyOperation, Vec<PayloadBytes>), String> {
    let mut operation = request.operation;
    if !matches!(
        operation.operation,
        ConversationApplyKind::ProviderMaterialization(_)
    ) {
        return Err("provider materialization requires a Pull apply operation".to_string());
    }
    if !operation.targets.is_empty() {
        return Err("new conversation operation must begin without prepared targets".to_string());
    }
    if request.targets.len() > MAX_CONVERSATION_FILES {
        return Err("conversation materialization has too many provider targets".to_string());
    }
    if operation.materialization_state == ConversationMaterializationState::Resumable
        && request.targets.is_empty()
    {
        return Err("resumable conversation materialization has no provider files".to_string());
    }
    verify_target_role_set(&operation, &request.targets)?;
    let operation_fragment = stable_fragment(operation.operation_id.as_str());
    let mut targets = Vec::with_capacity(request.targets.len());
    let mut payloads = Vec::with_capacity(request.targets.len());
    for (index, payload) in request.targets.into_iter().enumerate() {
        if payload.bytes.len() as u64 > MAX_PROVIDER_TARGET_BYTES {
            return Err("conversation provider target exceeds the write limit".to_string());
        }
        let target_path = canonical_target_path(&payload.target_path)?;
        let parent = target_path
            .parent()
            .ok_or_else(|| "conversation provider target has no parent".to_string())?;
        let file_name = target_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "conversation provider target has an invalid file name".to_string())?;
        let staged_path = parent.join(format!(
            ".{file_name}.mallard-{operation_fragment}-{index}.stage"
        ));
        let backup_path = matches!(payload.expected, TargetExpectation::Sha256(_)).then(|| {
            repository
                .root()
                .join("backups")
                .join("conversations")
                .join(operation.operation_id.as_str())
                .join(format!("{index}.bak"))
        });
        let staged_sha256 = payload.bytes.sha256().to_string();
        payloads.push(payload.bytes);
        targets.push(ConversationApplyTarget {
            target_path: path_text(&target_path)?,
            role: payload.role,
            expected: payload.expected,
            staged_path: Some(path_text(&staged_path)?),
            backup_path: backup_path
                .as_ref()
                .map(|path| path_text(path))
                .transpose()?,
            staged_sha256: Some(staged_sha256),
            index_rows: payload.index_rows,
        });
    }
    operation.targets = targets;
    operation.validate()?;
    Ok((operation, payloads))
}

fn validate_operation_scope_from_state(
    identity: &HostIdentity,
    machine: &MachineProjectState,
    bindings: &ConversationBindings,
    operation: &ConversationApplyOperation,
) -> Result<(), String> {
    if identity.host_id != operation.host_id
        || identity.revision != operation.expected_host_identity_revision
    {
        return Err("conversation operation belongs to a different or changed host".to_string());
    }
    let project_binding = machine
        .bindings
        .iter()
        .find(|binding| {
            binding.host_id == operation.host_id
                && binding.replica_id == operation.replica_id
                && binding.local_project_id == operation.local_project_id
        })
        .ok_or_else(|| "conversation operation references a missing project replica".to_string())?;
    if project_binding.revision != operation.expected_project_binding_revision
        || project_binding.profile_ids.get(&operation.provider)
            != Some(&operation.provider_profile_id)
    {
        return Err("conversation project binding changed after review".to_string());
    }
    let profile = machine
        .profiles
        .iter()
        .find(|profile| profile.profile_id == operation.provider_profile_id)
        .ok_or_else(|| {
            "conversation operation references a missing provider profile".to_string()
        })?;
    if profile.provider != operation.provider
        || profile.revision != operation.expected_profile_revision
    {
        return Err("conversation provider profile changed after review".to_string());
    }
    if bindings.revision != operation.expected_bindings_document_revision {
        return Err("conversation bindings changed after review".to_string());
    }
    Ok(())
}

fn ensure_provider_target_parents(
    machine: &MachineProjectState,
    operation: &ConversationApplyOperation,
    targets: &[ConversationTargetPayload],
) -> Result<(), String> {
    if targets.is_empty() {
        return Ok(());
    }
    let profile = machine
        .profiles
        .iter()
        .find(|profile| profile.profile_id == operation.provider_profile_id)
        .ok_or_else(|| {
            "conversation operation references a missing provider profile".to_string()
        })?;
    let root = PathBuf::from(&profile.canonical_path);
    let root_metadata = fs::symlink_metadata(&root)
        .map_err(|error| format!("inspect provider root '{}': {error}", root.display()))?;
    if !root_metadata.file_type().is_dir() || root_metadata.file_type().is_symlink() {
        return Err("conversation provider root is not a real directory".to_string());
    }
    let canonical_root = fs::canonicalize(&root)
        .map_err(|error| format!("resolve provider root '{}': {error}", root.display()))?;
    if canonical_root != root {
        return Err("conversation provider root is not canonical".to_string());
    }
    for target in targets {
        let parent = target
            .target_path
            .parent()
            .ok_or_else(|| "conversation provider target has no parent".to_string())?;
        let relative = parent.strip_prefix(&root).map_err(|_| {
            format!(
                "conversation provider target '{}' escapes its profile",
                target.target_path.display()
            )
        })?;
        let mut current = root.clone();
        for component in relative.components() {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(metadata)
                    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(format!(
                        "conversation provider directory '{}' is not a real directory",
                        current.display()
                    ))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(&current).map_err(|error| {
                        format!(
                            "create conversation provider directory '{}': {error}",
                            current.display()
                        )
                    })?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&current, fs::Permissions::from_mode(0o700)).map_err(
                            |error| {
                                format!(
                                    "secure conversation provider directory '{}': {error}",
                                    current.display()
                                )
                            },
                        )?;
                    }
                    sync_directory(current.parent().ok_or_else(|| {
                        "conversation provider directory has no parent".to_string()
                    })?)?;
                }
                Err(error) => {
                    return Err(format!(
                        "inspect conversation provider directory '{}': {error}",
                        current.display()
                    ))
                }
            }
        }
    }
    Ok(())
}

fn replaces_transcript(target: &ConversationApplyTarget) -> bool {
    target.role == ConversationFileRole::Transcript
        && matches!(target.expected, TargetExpectation::Sha256(_))
}

fn validate_target_expectations(
    targets: &[ConversationApplyTarget],
    checked_transcript: Option<&[u8]>,
) -> Result<(), String> {
    match (targets.iter().any(replaces_transcript), checked_transcript) {
        (true, Some(bytes)) if bytes.len() as u64 <= MAX_PROVIDER_TARGET_BYTES => {}
        (false, None) => {}
        _ => return Err("conversation replacement requires its checked source bytes".to_string()),
    }
    for target in targets {
        let path = Path::new(&target.target_path);
        if replaces_transcript(target) {
            // The source check already authorized replacement. Stat for
            // file-type and size safety, but do not read its contents again.
            inspect_regular_file(path)?;
            continue;
        }
        match (&target.expected, read_regular_file_digest(path)?) {
            (TargetExpectation::Absent, None) => {}
            (TargetExpectation::Sha256(expected), Some(actual)) if expected == &actual => {}
            _ => return Err("conversation provider target changed after review".to_string()),
        }
    }
    Ok(())
}

fn verify_target_role_set(
    operation: &ConversationApplyOperation,
    targets: &[ConversationTargetPayload],
) -> Result<(), String> {
    if operation.materialization_state == ConversationMaterializationState::StoredOnly {
        if !targets.is_empty() || operation.provider_thread_id.is_some() {
            return Err("stored-only conversation cannot publish provider files".to_string());
        }
        return Ok(());
    }
    if operation.materialization_state != ConversationMaterializationState::Resumable {
        return Err(
            "new conversation materialization must resolve to resumable or stored-only".to_string(),
        );
    }
    let transcripts = targets
        .iter()
        .filter(|target| target.role == ConversationFileRole::Transcript)
        .count();
    let indexes = targets
        .iter()
        .filter(|target| target.role == ConversationFileRole::ProviderIndexFragment)
        .count();
    if transcripts != 1 {
        return Err("resumable conversation requires exactly one transcript".to_string());
    }
    // Provider index rows are conditional presentation metadata (Codex keeps
    // `session_index.jsonl`); the registered adapter declares how many one
    // operation may carry. Providers without an index allow none.
    let adapter =
        crate::providers::AgentProviderRegistry::production().require(&operation.provider)?;
    if indexes > adapter.index_fragment_limit() {
        return Err(format!(
            "{} conversations allow at most {} index fragment(s)",
            operation.provider,
            adapter.index_fragment_limit()
        ));
    }
    Ok(())
}

fn verify_transcript_request(
    machine: &MachineProjectState,
    request: &ConversationMaterializationRequest,
) -> Result<(), String> {
    let operation = &request.operation;
    let ConversationApplyKind::ProviderMaterialization(materialization) = &operation.operation
    else {
        return Err("transcript outside a materialization".to_string());
    };
    let project = machine
        .bindings
        .iter()
        .find(|binding| {
            binding.host_id == operation.host_id
                && binding.replica_id == operation.replica_id
                && binding.local_project_id == operation.local_project_id
        })
        .ok_or("transcript project replica disappeared")?;
    let adapter =
        crate::providers::AgentProviderRegistry::production().require(&operation.provider)?;
    let target = request
        .targets
        .iter()
        .find(|target| target.role == ConversationFileRole::Transcript)
        .ok_or("materialization lacks its transcript")?;
    if !request.transcript.matches_target(
        &operation.provider,
        adapter.codec_key(),
        operation.provider_thread_id.as_deref(),
        &project.project_root,
        &project.canonical_project_root,
        &materialization.pinned_lineage_digest,
        &materialization.pinned_semantic_digest,
        &target.bytes,
    ) {
        return Err("verified transcript does not match the materialization target".to_string());
    }
    Ok(())
}

fn verify_provider_payload(
    operation: &ConversationApplyOperation,
    target: &ConversationApplyTarget,
    path: &Path,
) -> Result<(), String> {
    if target.role == ConversationFileRole::Transcript {
        // Commit checks the staged digest against the immutable verified bytes.
        return Ok(());
    }
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "read staged provider payload '{}': {}",
            path.display(),
            error
        )
    })?;
    // Provider payload shapes belong to the registered adapter; the core
    // keeps the boundary, digest, and no-clobber checks around them.
    let adapter =
        crate::providers::AgentProviderRegistry::production().require(&operation.provider)?;
    match &target.role {
        ConversationFileRole::Transcript => unreachable!("transcript verified before staging"),
        ConversationFileRole::ProviderIndexFragment => {
            // The prepared target names every row this fragment rewrites
            // (destination and, on keep-both, the preserved rename); the
            // domain guarantees the list is non-empty for this role.
            for thread_id in &target.index_rows {
                adapter.verify_index_fragment(&bytes, thread_id)?;
            }
        }
        ConversationFileRole::Sidecar { sidecar_kind } => {
            adapter.verify_sidecar(sidecar_kind, &bytes)?;
        }
    }
    Ok(())
}

fn commit_one_target(target: &ConversationApplyTarget) -> Result<(), String> {
    let path = Path::new(&target.target_path);
    let stage = Path::new(
        target
            .staged_path
            .as_deref()
            .ok_or_else(|| "conversation target lacks a stage".to_string())?,
    );
    let staged_digest = target
        .staged_sha256
        .as_deref()
        .ok_or_else(|| "conversation target lacks a digest".to_string())?;
    if read_regular_file_digest(stage)?.as_deref() != Some(staged_digest) {
        return Err("conversation staged provider target is unavailable".to_string());
    }
    ensure_canonical_parent(path)?;
    match &target.expected {
        TargetExpectation::Absent => {
            if inspect_regular_file(path)? {
                return Err("conversation provider target changed during apply".to_string());
            }
            fs::hard_link(stage, path).map_err(|error| {
                format!(
                    "publish no-clobber conversation target '{}': {}",
                    path.display(),
                    error
                )
            })?;
        }
        TargetExpectation::Sha256(expected) => {
            if replaces_transcript(target) {
                // The one source check permits a missed transcript update.
                // Shared provider files still require their current digest.
                inspect_regular_file(path)?;
            } else if read_regular_file_digest(path)?.as_ref() != Some(expected) {
                return Err("conversation provider target changed during apply".to_string());
            }
            fs::rename(stage, path).map_err(|error| {
                format!(
                    "replace conversation target '{}': {}",
                    path.display(),
                    error
                )
            })?;
        }
    }
    sync_directory(
        path.parent()
            .ok_or_else(|| "conversation target has no parent".to_string())?,
    )
}

fn transcript_target_digest(operation: &ConversationApplyOperation) -> Option<String> {
    operation
        .targets
        .iter()
        .find(|target| target.role == ConversationFileRole::Transcript)
        .and_then(|target| target.staged_sha256.clone())
}

fn prepare_receipt(
    operation: &ConversationApplyOperation,
    cloud_revision: u64,
    cloud_commit_id: &str,
    cloud_manifest_sha256: &str,
) -> Result<(ConversationApplyReceipt, MaterializationRecord), String> {
    let receipt = conversation_receipt(
        operation,
        cloud_revision,
        cloud_commit_id,
        cloud_manifest_sha256,
    );
    let plan_id = operation
        .plan_id
        .clone()
        .ok_or_else(|| "conversation operation lacks a plan id".to_string())?;
    let action_id = operation
        .plan_action_id
        .clone()
        .map(Ok)
        .unwrap_or_else(|| stable_action_id(&operation.operation_id))?;
    let apply_receipt = ApplyReceipt {
        action_id,
        resource_id: conversation_resource_id(&operation.provider, &operation.thread_id)?,
        action_type: match operation.direction() {
            ConversationPlanDirection::Push => RestoreActionType::PublishConversation,
            ConversationPlanDirection::Pull => RestoreActionType::MaterializeConversation,
        },
        logical_path: None,
        source_sha256: None,
        target_path: operation
            .targets
            .iter()
            .find(|target| target.role == ConversationFileRole::Transcript)
            .map(|target| target.target_path.clone()),
        target_sha256_after: receipt.local_file_sha256_after.clone(),
        status: ActionStatus::Applied,
        applied_at: operation.updated_at,
        error: None,
        conversation: Some(receipt.clone()),
    };
    let record = MaterializationRecord {
        materialization_id: stable_materialization_id(&operation.operation_id)?,
        plan_id,
        host_id: operation.host_id.clone(),
        replica_id: operation.replica_id.clone(),
        local_project_id: operation.local_project_id.clone(),
        storage_id: operation.cloud.storage_id.clone(),
        bundle_id: operation.cloud.bundle_id.clone(),
        revision: cloud_revision,
        commit_id: cloud_commit_id.to_string(),
        manifest_sha256: cloud_manifest_sha256.to_string(),
        binding_revision: operation.expected_project_binding_revision,
        status: MaterializationStatus::Complete,
        applied_at: operation.updated_at,
        receipts: vec![apply_receipt],
    };
    Ok((receipt, record))
}

fn insert_receipt_record(
    materializations: &mut Materializations,
    mut incoming: MaterializationRecord,
) -> Result<(), String> {
    let operation_id = &incoming.receipts[0]
        .conversation
        .as_ref()
        .expect("prepared record has a conversation receipt")
        .operation_id;
    if materializations.records.iter().any(|record| {
        record.receipts.iter().any(|receipt| {
            receipt
                .conversation
                .as_ref()
                .is_some_and(|existing| &existing.operation_id == operation_id)
        })
    }) {
        return Ok(());
    }
    if let Some(record) = materializations
        .records
        .iter_mut()
        .find(|record| record.plan_id == incoming.plan_id)
    {
        if record.host_id != incoming.host_id
            || record.replica_id != incoming.replica_id
            || record.local_project_id != incoming.local_project_id
            || record.storage_id != incoming.storage_id
            || record.bundle_id != incoming.bundle_id
        {
            return Err(
                "conversation receipt plan scope disagrees with materialization".to_string(),
            );
        }
        record.receipts.append(&mut incoming.receipts);
        record.applied_at = record.applied_at.max(incoming.applied_at);
    } else {
        materializations.records.push(incoming);
    }
    materializations.schema = LOCAL_SCHEMA;
    Ok(())
}

fn conversation_receipt(
    operation: &ConversationApplyOperation,
    cloud_revision: u64,
    cloud_commit_id: &str,
    cloud_manifest_sha256: &str,
) -> ConversationApplyReceipt {
    let (accepted_tail, preserved_thread_id, preserved_display_title, source_disposition) =
        match &operation.operation {
            ConversationApplyKind::CloudPublication(publication) => (
                Some(publication.tail_after.clone()),
                None,
                None,
                Some(ConversationSourceDisposition::Retained),
            ),
            ConversationApplyKind::ProviderMaterialization(materialization) => (
                Some(materialization.accepted_tail_after.clone()),
                materialization
                    .keep_both
                    .as_ref()
                    .map(|keep_both| keep_both.preserved_thread_id.clone()),
                materialization
                    .keep_both
                    .as_ref()
                    .map(|keep_both| keep_both.preserved_display_title.clone()),
                Some(ConversationSourceDisposition::Retained),
            ),
        };
    ConversationApplyReceipt {
        host_id: operation.host_id.clone(),
        replica_id: operation.replica_id.clone(),
        thread_id: operation.thread_id.clone(),
        provider_profile_id: operation.provider_profile_id.clone(),
        provider_thread_id: operation.provider_thread_id.clone(),
        materialization_state: operation.materialization_state,
        lineage_digest: None,
        semantic_digest: None,
        local_file_sha256_after: transcript_target_digest(operation),
        cloud_revision,
        cloud_commit_id: cloud_commit_id.to_string(),
        cloud_manifest_sha256: cloud_manifest_sha256.to_string(),
        published_origin_host_token: None,
        published_origin_replica_token: None,
        operation_id: operation.operation_id.clone(),
        accepted_tail,
        preserved_thread_id,
        preserved_display_title,
        source_disposition,
    }
}

fn canonical_target_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("conversation provider target must be absolute".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "conversation provider target has no parent".to_string())?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("resolve conversation target parent: {error}"))?;
    if canonical_parent != parent {
        return Err("conversation provider target parent is not canonical".to_string());
    }
    Ok(canonical_parent.join(
        path.file_name()
            .ok_or_else(|| "conversation provider target has no file name".to_string())?,
    ))
}

fn ensure_canonical_parent(path: &Path) -> Result<(), String> {
    canonical_target_path(path).map(|_| ())
}

fn inspect_regular_file(path: &Path) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("inspect '{}': {}", path.display(), error)),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(format!(
            "'{}' is not a regular no-follow file",
            path.display()
        ));
    }
    if metadata.len() > MAX_PROVIDER_TARGET_BYTES {
        return Err(format!(
            "'{}' exceeds the provider target limit",
            path.display()
        ));
    }
    Ok(true)
}

fn read_regular_file_digest(path: &Path) -> Result<Option<String>, String> {
    if !inspect_regular_file(path)? {
        return Ok(None);
    }
    let mut file =
        File::open(path).map_err(|error| format!("open '{}': {}", path.display(), error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("read '{}': {}", path.display(), error))?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > MAX_PROVIDER_TARGET_BYTES {
            return Err(format!(
                "'{}' exceeds the provider target limit",
                path.display()
            ));
        }
        hasher.update(&buffer[..count]);
    }
    crate::sync_metrics::record_transcript_bytes_read(path, total);
    crate::sync_metrics::record_bytes_hashed(path, total);
    Ok(Some(hex_lower(&hasher.finalize())))
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("create '{}': {}", path.display(), error))?;
    file.write_all(bytes)
        .map_err(|error| format!("write '{}': {}", path.display(), error))?;
    file.sync_all()
        .map_err(|error| format!("sync '{}': {}", path.display(), error))?;
    sync_directory(
        path.parent()
            .ok_or_else(|| "written file has no parent".to_string())?,
    )
}

fn copy_new_file(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("inspect '{}': {}", source.display(), error))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(format!(
            "'{}' is not a regular source file",
            source.display()
        ));
    }
    if metadata.len() > MAX_PROVIDER_TARGET_BYTES {
        return Err(format!(
            "'{}' exceeds the provider target limit",
            source.display()
        ));
    }
    let bytes =
        fs::read(source).map_err(|error| format!("read '{}': {}", source.display(), error))?;
    crate::sync_metrics::record_transcript_bytes_read(source, bytes.len() as u64);
    write_new_file(target, &bytes)
}

fn create_private_directory_all(root: &Path, path: &Path) -> Result<(), String> {
    if !path.starts_with(root) {
        return Err("conversation private directory escapes Mallard state".to_string());
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "conversation private directory escapes Mallard state".to_string())?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err(format!("'{}' is not a real directory", current.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|error| format!("create '{}': {}", current.display(), error))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&current, fs::Permissions::from_mode(0o700))
                        .map_err(|error| format!("secure '{}': {}", current.display(), error))?;
                }
                sync_directory(
                    current
                        .parent()
                        .ok_or_else(|| "private directory has no parent".to_string())?,
                )?;
            }
            Err(error) => return Err(format!("inspect '{}': {}", current.display(), error)),
        }
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    let directory = File::open(path)
        .map_err(|error| format!("open directory '{}': {}", path.display(), error))?;
    directory
        .sync_all()
        .map_err(|error| format!("sync directory '{}': {}", path.display(), error))
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| "conversation path is not valid UTF-8".to_string())
}

fn stable_fragment(value: &str) -> String {
    sha256_hex(value.as_bytes())[..24].to_string()
}

fn stable_action_id(operation_id: &ConversationOperationId) -> Result<ActionId, String> {
    ActionId::parse(format!(
        "action-conversation-{}",
        stable_fragment(operation_id.as_str())
    ))
}

fn stable_materialization_id(
    operation_id: &ConversationOperationId,
) -> Result<MaterializationId, String> {
    MaterializationId::parse(format!(
        "materialization-conversation-{}",
        stable_fragment(operation_id.as_str())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(
        root: &Path,
        name: &str,
        role: ConversationFileRole,
        existing: Option<&[u8]>,
    ) -> ConversationApplyTarget {
        let path = root.join(name);
        if let Some(bytes) = existing {
            fs::write(&path, bytes).unwrap();
        }
        let stage = root.join(format!("{name}.stage"));
        fs::write(&stage, b"verified cloud payload").unwrap();
        let index_rows = if role == ConversationFileRole::ProviderIndexFragment {
            vec!["thread-one".to_string()]
        } else {
            Vec::new()
        };
        ConversationApplyTarget {
            target_path: path_text(&path).unwrap(),
            role,
            expected: existing
                .map(|bytes| TargetExpectation::Sha256(sha256_hex(bytes)))
                .unwrap_or(TargetExpectation::Absent),
            staged_path: Some(path_text(&stage).unwrap()),
            backup_path: existing.map(|_| path_text(&root.join(format!("{name}.bak"))).unwrap()),
            staged_sha256: Some(sha256_hex(b"verified cloud payload")),
            index_rows,
        }
    }

    #[test]
    fn checked_transcript_is_replaced_without_another_source_read() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let transcript = target(
            &root,
            "source",
            ConversationFileRole::Transcript,
            Some(b"captured"),
        );
        let metrics = crate::sync_metrics::register_root(Path::new(&transcript.target_path));
        assert!(validate_target_expectations(&[transcript.clone()], None).is_err());
        fs::write(&transcript.target_path, b"changed after the source check").unwrap();
        validate_target_expectations(&[transcript.clone()], Some(b"captured")).unwrap();
        commit_one_target(&transcript).unwrap();
        assert_eq!(
            fs::read(&transcript.target_path).unwrap(),
            b"verified cloud payload"
        );
        assert_eq!(metrics.snapshot().transcript_bytes_read, 0);
        assert_eq!(metrics.snapshot().bytes_hashed, 0);
    }

    #[test]
    fn shared_index_conflicts_still_fail_before_staging_and_at_commit() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let transcript = target(
            &root,
            "source",
            ConversationFileRole::Transcript,
            Some(b"captured"),
        );
        let index = target(
            &root,
            "index",
            ConversationFileRole::ProviderIndexFragment,
            Some(b"old index"),
        );
        let targets = [transcript, index.clone()];
        validate_target_expectations(&targets, Some(b"captured")).unwrap();
        fs::write(&index.target_path, b"updated unrelated thread entry").unwrap();
        assert!(validate_target_expectations(&targets, Some(b"captured")).is_err());
        assert!(commit_one_target(&index).is_err());
        assert_eq!(
            fs::read(&index.target_path).unwrap(),
            b"updated unrelated thread entry"
        );
    }

    #[test]
    fn new_transcript_destinations_still_reject_occupants() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let transcript = target(&root, "source", ConversationFileRole::Transcript, None);
        assert!(validate_target_expectations(&[transcript.clone()], Some(b"unneeded")).is_err());
        validate_target_expectations(&[transcript.clone()], None).unwrap();
        fs::write(&transcript.target_path, b"another thread").unwrap();
        assert!(validate_target_expectations(&[transcript.clone()], None).is_err());
        assert!(commit_one_target(&transcript).is_err());
        assert_eq!(
            fs::read(&transcript.target_path).unwrap(),
            b"another thread"
        );
    }

    #[test]
    fn checked_transcript_does_not_bypass_staged_payload_verification() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let transcript = target(
            &root,
            "source",
            ConversationFileRole::Transcript,
            Some(b"captured"),
        );
        validate_target_expectations(&[transcript.clone()], Some(b"captured")).unwrap();
        fs::write(
            transcript.staged_path.as_deref().unwrap(),
            b"corrupted stage",
        )
        .unwrap();
        assert!(commit_one_target(&transcript).is_err());
        assert_eq!(fs::read(&transcript.target_path).unwrap(), b"captured");
    }

    #[test]
    fn checked_transcript_still_rejects_directories_and_oversized_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let transcript = target(
            &root,
            "source",
            ConversationFileRole::Transcript,
            Some(b"captured"),
        );
        validate_target_expectations(&[transcript.clone()], Some(b"captured")).unwrap();
        fs::remove_file(&transcript.target_path).unwrap();
        fs::create_dir(&transcript.target_path).unwrap();
        assert!(commit_one_target(&transcript).is_err());
        fs::remove_dir(&transcript.target_path).unwrap();
        File::create(&transcript.target_path)
            .unwrap()
            .set_len(MAX_PROVIDER_TARGET_BYTES + 1)
            .unwrap();
        assert!(commit_one_target(&transcript).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn checked_transcript_still_rejects_a_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let transcript = target(
            &root,
            "source",
            ConversationFileRole::Transcript,
            Some(b"captured"),
        );
        validate_target_expectations(&[transcript.clone()], Some(b"captured")).unwrap();
        let unrelated = root.join("unrelated");
        fs::write(&unrelated, b"unrelated content").unwrap();
        fs::remove_file(&transcript.target_path).unwrap();
        std::os::unix::fs::symlink(&unrelated, &transcript.target_path).unwrap();
        assert!(commit_one_target(&transcript).is_err());
        assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated content");
    }
}
