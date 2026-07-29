//! Crash-consistent provider-file materialization for reviewed conversation
//! actions.
//!
//! Provider transcripts, indexes, sidecars, bindings, and receipts cannot be
//! committed by one filesystem rename. This module uses the durable
//! conversation journal as the source of truth and makes every step
//! restartable and idempotent.

use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::conversation_codec::{analyze_codex_transcript, CodecLineage};
use super::conversation_sync::{rekey_thread_binding, ConversationBindingScope};
use super::domain::{
    conversation_resource_id, ActionId, ActionStatus, ApplyReceipt, ConversationApplyReceipt,
    ConversationFileRole, ConversationJournalTarget, ConversationMaterializationState,
    ConversationOperationId, ConversationPlanDirection, ConversationSyncOperation,
    ConversationSyncPhase, LocalConversationBinding, MaterializationId, MaterializationRecord,
    MaterializationStatus, RestoreActionType, TargetExpectation, LOCAL_SCHEMA,
};
use super::persistence::LocalStateRepository;

const MAX_PROVIDER_TARGET_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationTargetPayload {
    pub target_path: PathBuf,
    pub role: ConversationFileRole,
    pub expected: TargetExpectation,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ConversationMaterializationRequest {
    pub operation: ConversationSyncOperation,
    pub targets: Vec<ConversationTargetPayload>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationFailpoint {
    BeforePrepared,
    AfterPrepared,
    BeforeStage {
        index: usize,
        role: ConversationFileRole,
    },
    AfterStage {
        index: usize,
        role: ConversationFileRole,
    },
    BeforeStaged,
    AfterStaged,
    BeforeLocalRekey,
    AfterLocalRekey,
    BeforeCloudPublication,
    AfterCloudHeadCas,
    BeforeCloudPublished,
    AfterCloudPublished,
    BeforeTargetCommit {
        index: usize,
        role: ConversationFileRole,
    },
    AfterTargetWrite {
        index: usize,
        role: ConversationFileRole,
    },
    AfterTargetCommit {
        index: usize,
        role: ConversationFileRole,
    },
    BeforeProviderCommitted,
    AfterProviderCommitted,
    BeforeBindingCommit,
    AfterBindingCommit,
    BeforeReceipt,
    AfterReceipt,
    BeforeLocalCommitted,
    AfterLocalCommitted,
    BeforeCleanup,
    AfterCleanup,
}

pub trait ConversationFailpoints {
    fn hit(&mut self, point: &ConversationFailpoint) -> Result<(), String>;
}

#[derive(Default)]
pub struct NoConversationFailpoints;

impl ConversationFailpoints for NoConversationFailpoints {
    fn hit(&mut self, _point: &ConversationFailpoint) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConversationRecoveryReport {
    pub completed: Vec<ConversationOperationId>,
    pub rolled_back: Vec<ConversationOperationId>,
    pub recovery_required: Vec<ConversationOperationId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationCloudPublication {
    pub generation: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
}

/// Journal a reviewed Push, durably rekey a same-ID local fork when needed,
/// publish through the caller's production bundle engine, and finalize the
/// local receipt. `inspect_after_error` closes the important CAS ambiguity:
/// an object store may commit the head and then return an I/O error.
#[cfg(test)]
pub fn apply_conversation_cloud_publication(
    repository: &LocalStateRepository,
    mut operation: ConversationSyncOperation,
    publish: impl FnOnce() -> Result<ConversationCloudPublication, String>,
    inspect_after_error: impl FnOnce() -> Result<Option<ConversationCloudPublication>, String>,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<ConversationApplyReceipt, String> {
    let _host_guard = repository.acquire_host_operation()?;
    if operation.direction != ConversationPlanDirection::Push
        || operation.phase != ConversationSyncPhase::Prepared
        || !operation.targets.is_empty()
    {
        return Err("cloud publication requires a new Prepared Push operation".to_string());
    }
    operation.validate()?;
    validate_operation_scope(repository, &operation)?;
    failpoints.hit(&ConversationFailpoint::BeforePrepared)?;
    insert_operation(repository, &operation)?;
    failpoints.hit(&ConversationFailpoint::AfterPrepared)?;

    failpoints.hit(&ConversationFailpoint::BeforeCloudPublication)?;
    let publication = match publish() {
        Ok(publication) => publication,
        Err(error) => match inspect_after_error()? {
            Some(publication) => publication,
            None => {
                cleanup_and_retire(repository, &operation.operation_id)?;
                return Err(error);
            }
        },
    };
    failpoints.hit(&ConversationFailpoint::AfterCloudHeadCas)?;
    failpoints.hit(&ConversationFailpoint::BeforeCloudPublished)?;
    let (_, updated) = mutate_operation(repository, &operation.operation_id, |operation| {
        operation.generation = publication.generation;
        operation.commit_id = publication.commit_id.clone();
        operation.manifest_sha256 = publication.manifest_sha256.clone();
        operation.phase = ConversationSyncPhase::CloudPublished;
        Ok(())
    })?;
    operation = updated;
    failpoints.hit(&ConversationFailpoint::AfterCloudPublished)?;

    // Preserve the existing binding until immutable publication and the head
    // CAS are proven complete. Recovery can safely finish this exact rekey
    // from the journaled result ID after a post-CAS crash.
    operation = commit_rekey(repository, &operation)?;
    failpoints.hit(&ConversationFailpoint::BeforeBindingCommit)?;
    commit_binding(repository, &operation)?;
    failpoints.hit(&ConversationFailpoint::AfterBindingCommit)?;
    failpoints.hit(&ConversationFailpoint::BeforeReceipt)?;
    let receipt = commit_receipt(repository, &operation)?;
    failpoints.hit(&ConversationFailpoint::AfterReceipt)?;
    failpoints.hit(&ConversationFailpoint::BeforeLocalCommitted)?;
    set_phase(
        repository,
        &operation.operation_id,
        ConversationSyncPhase::LocalCommitted,
        operation.updated_at,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterLocalCommitted)?;
    failpoints.hit(&ConversationFailpoint::BeforeCleanup)?;
    cleanup_and_retire(repository, &operation.operation_id)?;
    failpoints.hit(&ConversationFailpoint::AfterCleanup)?;
    Ok(receipt)
}

/// Publish a reviewed set of Push conversation actions while the command
/// orchestrator holds the host-operation guard across generic and
/// conversation publication.
pub(crate) fn apply_conversation_cloud_publications_locked(
    repository: &LocalStateRepository,
    operations: Vec<ConversationSyncOperation>,
    publish: impl FnOnce() -> Result<ConversationCloudPublication, String>,
    inspect_after_error: impl FnOnce() -> Result<Option<ConversationCloudPublication>, String>,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<Vec<ConversationApplyReceipt>, String> {
    if operations.is_empty() {
        return Err("cloud publication requires at least one conversation action".to_string());
    }
    let mut operation_ids = std::collections::BTreeSet::new();
    let mut result_threads = std::collections::BTreeSet::new();
    let first = &operations[0];
    for operation in &operations {
        if operation.direction != ConversationPlanDirection::Push
            || operation.phase != ConversationSyncPhase::Prepared
            || !operation.targets.is_empty()
        {
            return Err("cloud publication requires Prepared Push operations".to_string());
        }
        operation.validate()?;
        validate_operation_scope(repository, operation)?;
        if operation.plan_id != first.plan_id
            || operation.storage_id != first.storage_id
            || operation.bundle_id != first.bundle_id
            || operation.generation != first.generation
            || operation.commit_id != first.commit_id
            || operation.manifest_sha256 != first.manifest_sha256
            || operation.host_id != first.host_id
            || operation.replica_id != first.replica_id
            || operation.local_project_id != first.local_project_id
        {
            return Err("conversation publication batch has inconsistent plan pins".to_string());
        }
        if !operation_ids.insert(operation.operation_id.clone())
            || !result_threads.insert(operation.result_thread_id.clone())
        {
            return Err(
                "conversation publication batch repeats an operation or result".to_string(),
            );
        }
    }

    failpoints.hit(&ConversationFailpoint::BeforePrepared)?;
    insert_operations(repository, &operations)?;
    failpoints.hit(&ConversationFailpoint::AfterPrepared)?;
    failpoints.hit(&ConversationFailpoint::BeforeCloudPublication)?;
    let publication = match publish() {
        Ok(publication) => publication,
        Err(error) => match inspect_after_error()? {
            Some(publication) => publication,
            None => {
                for operation in &operations {
                    cleanup_and_retire(repository, &operation.operation_id)?;
                }
                return Err(error);
            }
        },
    };
    failpoints.hit(&ConversationFailpoint::AfterCloudHeadCas)?;
    failpoints.hit(&ConversationFailpoint::BeforeCloudPublished)?;
    let journal = repository.load_conversation_sync_journal()?;
    repository.mutate_conversation_sync_journal(journal.revision, |journal| {
        for operation_id in &operation_ids {
            let operation = journal
                .operations
                .iter_mut()
                .find(|operation| &operation.operation_id == operation_id)
                .ok_or_else(|| format!("conversation operation '{}' disappeared", operation_id))?;
            operation.generation = publication.generation;
            operation.commit_id = publication.commit_id.clone();
            operation.manifest_sha256 = publication.manifest_sha256.clone();
            operation.phase = ConversationSyncPhase::CloudPublished;
        }
        Ok(())
    })?;
    failpoints.hit(&ConversationFailpoint::AfterCloudPublished)?;

    let mut receipts = Vec::with_capacity(operation_ids.len());
    for operation_id in operation_ids {
        let operation = repository
            .load_conversation_sync_journal()?
            .operations
            .into_iter()
            .find(|operation| operation.operation_id == operation_id)
            .ok_or_else(|| format!("conversation operation '{}' disappeared", operation_id))?;
        let operation = rebase_operation_after_batch_progress(repository, &operation)?;
        let operation = commit_rekey(repository, &operation)?;
        failpoints.hit(&ConversationFailpoint::BeforeBindingCommit)?;
        commit_binding(repository, &operation)?;
        failpoints.hit(&ConversationFailpoint::AfterBindingCommit)?;
        set_phase(
            repository,
            &operation.operation_id,
            ConversationSyncPhase::BindingsCommitted,
            operation.updated_at,
        )?;
        failpoints.hit(&ConversationFailpoint::BeforeReceipt)?;
        receipts.push(commit_receipt(repository, &operation)?);
        failpoints.hit(&ConversationFailpoint::AfterReceipt)?;
        failpoints.hit(&ConversationFailpoint::BeforeLocalCommitted)?;
        set_phase(
            repository,
            &operation.operation_id,
            ConversationSyncPhase::LocalCommitted,
            operation.updated_at,
        )?;
        failpoints.hit(&ConversationFailpoint::AfterLocalCommitted)?;
        failpoints.hit(&ConversationFailpoint::BeforeCleanup)?;
        cleanup_and_retire(repository, &operation.operation_id)?;
        failpoints.hit(&ConversationFailpoint::AfterCleanup)?;
    }
    Ok(receipts)
}

/// Recover Push operations with a caller-supplied immutable-head inspector.
/// The inspector must return a publication only after validating that the
/// reviewed result variant and digests are present in that exact manifest.
#[cfg(test)]
pub fn recover_conversation_cloud_publications(
    repository: &LocalStateRepository,
    mut inspect: impl FnMut(
        &ConversationSyncOperation,
    ) -> Result<Option<ConversationCloudPublication>, String>,
) -> Result<ConversationRecoveryReport, String> {
    let _host_guard = repository.acquire_host_operation()?;
    let mut report = ConversationRecoveryReport::default();
    let operations = repository.load_conversation_sync_journal()?.operations;
    for mut operation in operations
        .into_iter()
        .filter(|operation| operation.direction == ConversationPlanDirection::Push)
    {
        let operation_id = operation.operation_id.clone();
        if operation.phase == ConversationSyncPhase::Prepared {
            match inspect(&operation)? {
                Some(publication) => {
                    let (_, updated) = mutate_operation(repository, &operation_id, |operation| {
                        operation.generation = publication.generation;
                        operation.commit_id = publication.commit_id.clone();
                        operation.manifest_sha256 = publication.manifest_sha256.clone();
                        operation.phase = ConversationSyncPhase::CloudPublished;
                        Ok(())
                    })?;
                    operation = updated;
                }
                None => {
                    cleanup_and_retire(repository, &operation_id)?;
                    report.rolled_back.push(operation_id);
                    continue;
                }
            }
        }
        if operation.phase == ConversationSyncPhase::RecoveryRequired {
            if let Some(publication) = inspect(&operation)? {
                let (_, updated) = mutate_operation(repository, &operation_id, |operation| {
                    operation.generation = publication.generation;
                    operation.commit_id = publication.commit_id.clone();
                    operation.manifest_sha256 = publication.manifest_sha256.clone();
                    operation.phase = ConversationSyncPhase::CloudPublished;
                    Ok(())
                })?;
                operation = updated;
            } else {
                report.recovery_required.push(operation_id);
                continue;
            }
        }
        if operation.phase == ConversationSyncPhase::CloudPublished {
            operation = rebase_operation_after_batch_progress(repository, &operation)?;
            operation = commit_rekey(repository, &operation)?;
            commit_binding(repository, &operation)?;
            operation = set_phase(
                repository,
                &operation_id,
                ConversationSyncPhase::BindingsCommitted,
                operation.updated_at,
            )?;
        }
        if operation.phase == ConversationSyncPhase::BindingsCommitted {
            commit_receipt(repository, &operation)?;
            operation = set_phase(
                repository,
                &operation_id,
                ConversationSyncPhase::LocalCommitted,
                operation.updated_at,
            )?;
        }
        if operation.phase == ConversationSyncPhase::LocalCommitted {
            cleanup_and_retire(repository, &operation_id)?;
        }
        report.completed.push(operation_id);
    }
    Ok(report)
}

/// Apply one reviewed Pull materialization. All target expectations and local
/// scope revisions are checked before the Prepared record is written.
#[cfg(test)]
pub fn apply_conversation_materialization(
    repository: &LocalStateRepository,
    request: ConversationMaterializationRequest,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<ConversationApplyReceipt, String> {
    let _host_guard = repository.acquire_host_operation()?;
    apply_conversation_materialization_locked(repository, request, failpoints)
}

pub(crate) fn apply_conversation_materialization_locked(
    repository: &LocalStateRepository,
    request: ConversationMaterializationRequest,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<ConversationApplyReceipt, String> {
    request.operation.validate()?;
    validate_operation_scope(repository, &request.operation)?;
    ensure_provider_target_parents(repository, &request.operation, &request.targets)?;
    let (mut operation, payloads) = prepare_operation(repository, request)?;
    validate_target_expectations(&operation.targets)?;

    failpoints.hit(&ConversationFailpoint::BeforePrepared)?;
    insert_operation(repository, &operation)?;
    failpoints.hit(&ConversationFailpoint::AfterPrepared)?;

    for (index, bytes) in payloads.iter().enumerate() {
        write_intent_payload(repository, &operation.operation_id, index, bytes)?;
    }

    stage_targets(repository, &operation, failpoints)?;
    failpoints.hit(&ConversationFailpoint::BeforeStaged)?;
    operation = set_phase(
        repository,
        &operation.operation_id,
        ConversationSyncPhase::Staged,
        operation.updated_at,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterStaged)?;

    if operation.is_composite_pull_fork() {
        failpoints.hit(&ConversationFailpoint::BeforeLocalRekey)?;
        operation = commit_rekey(repository, &operation)?;
        failpoints.hit(&ConversationFailpoint::AfterLocalRekey)?;
        operation = set_phase(
            repository,
            &operation.operation_id,
            ConversationSyncPhase::LocalRekeyed,
            operation.updated_at,
        )?;
    }

    commit_provider_targets(repository, &operation.operation_id, failpoints)?;
    failpoints.hit(&ConversationFailpoint::BeforeProviderCommitted)?;
    operation = set_phase(
        repository,
        &operation.operation_id,
        ConversationSyncPhase::ProviderCommitted,
        operation.updated_at,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterProviderCommitted)?;

    failpoints.hit(&ConversationFailpoint::BeforeBindingCommit)?;
    commit_binding(repository, &operation)?;
    failpoints.hit(&ConversationFailpoint::AfterBindingCommit)?;
    operation = set_phase(
        repository,
        &operation.operation_id,
        ConversationSyncPhase::BindingsCommitted,
        operation.updated_at,
    )?;

    failpoints.hit(&ConversationFailpoint::BeforeReceipt)?;
    let receipt = commit_receipt(repository, &operation)?;
    failpoints.hit(&ConversationFailpoint::AfterReceipt)?;

    failpoints.hit(&ConversationFailpoint::BeforeLocalCommitted)?;
    set_phase(
        repository,
        &operation.operation_id,
        ConversationSyncPhase::LocalCommitted,
        operation.updated_at,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterLocalCommitted)?;

    failpoints.hit(&ConversationFailpoint::BeforeCleanup)?;
    cleanup_and_retire(repository, &operation.operation_id)?;
    failpoints.hit(&ConversationFailpoint::AfterCleanup)?;
    Ok(receipt)
}

/// Recover every local operation after reopening the repository. Calling this
/// twice is intentionally a supported fixed-point check.
pub fn recover_conversation_materializations(
    repository: &LocalStateRepository,
) -> Result<ConversationRecoveryReport, String> {
    let _host_guard = repository.acquire_host_operation()?;
    let mut report = ConversationRecoveryReport::default();
    let operations = repository.load_conversation_sync_journal()?.operations;
    for mut operation in operations {
        let operation_id = operation.operation_id.clone();
        let result = (|| -> Result<RecoveryOutcome, String> {
            if operation.direction == ConversationPlanDirection::Push {
                mark_recovery_required(repository, &operation_id)?;
                return Ok(RecoveryOutcome::RecoveryRequired);
            }
            match operation.phase {
                ConversationSyncPhase::Prepared => {
                    if operation.targets.iter().all(staged_target_is_valid) {
                        operation = set_phase(
                            repository,
                            &operation_id,
                            ConversationSyncPhase::Staged,
                            operation.updated_at,
                        )?;
                    } else if operation.targets.iter().all(target_is_uncommitted) {
                        rollback_prepared(repository, &operation)?;
                        return Ok(RecoveryOutcome::RolledBack);
                    } else {
                        mark_recovery_required(repository, &operation_id)?;
                        return Ok(RecoveryOutcome::RecoveryRequired);
                    }
                }
                ConversationSyncPhase::RecoveryRequired => {
                    return Ok(RecoveryOutcome::RecoveryRequired)
                }
                _ => {}
            }

            if operation.phase == ConversationSyncPhase::Staged {
                if operation.is_composite_pull_fork() {
                    operation = commit_rekey(repository, &operation)?;
                    operation = set_phase(
                        repository,
                        &operation_id,
                        ConversationSyncPhase::LocalRekeyed,
                        operation.updated_at,
                    )?;
                }
            }
            if operation.phase == ConversationSyncPhase::Staged {
                let mut no_failpoints = NoConversationFailpoints;
                commit_provider_targets(repository, &operation_id, &mut no_failpoints)?;
                operation = set_phase(
                    repository,
                    &operation_id,
                    ConversationSyncPhase::ProviderCommitted,
                    operation.updated_at,
                )?;
            }
            if operation.phase == ConversationSyncPhase::LocalRekeyed {
                let mut no_failpoints = NoConversationFailpoints;
                commit_provider_targets(repository, &operation_id, &mut no_failpoints)?;
                operation = set_phase(
                    repository,
                    &operation_id,
                    ConversationSyncPhase::ProviderCommitted,
                    operation.updated_at,
                )?;
            }
            if matches!(
                operation.phase,
                ConversationSyncPhase::ProviderCommitted | ConversationSyncPhase::CloudPublished
            ) {
                commit_binding(repository, &operation)?;
                operation = set_phase(
                    repository,
                    &operation_id,
                    ConversationSyncPhase::BindingsCommitted,
                    operation.updated_at,
                )?;
            }
            if operation.phase == ConversationSyncPhase::BindingsCommitted {
                commit_receipt(repository, &operation)?;
                operation = set_phase(
                    repository,
                    &operation_id,
                    ConversationSyncPhase::LocalCommitted,
                    operation.updated_at,
                )?;
            }
            if operation.phase == ConversationSyncPhase::LocalCommitted {
                cleanup_and_retire(repository, &operation_id)?;
            }
            Ok(RecoveryOutcome::Completed)
        })();
        match result {
            Ok(RecoveryOutcome::Completed) => report.completed.push(operation_id),
            Ok(RecoveryOutcome::RolledBack) => report.rolled_back.push(operation_id),
            Ok(RecoveryOutcome::RecoveryRequired) => report.recovery_required.push(operation_id),
            Err(error) => {
                let _ = mark_recovery_required(repository, &operation_id);
                report.recovery_required.push(operation_id);
                if error.contains("different host") {
                    continue;
                }
            }
        }
    }
    Ok(report)
}

enum RecoveryOutcome {
    Completed,
    RolledBack,
    RecoveryRequired,
}

fn prepare_operation(
    repository: &LocalStateRepository,
    request: ConversationMaterializationRequest,
) -> Result<(ConversationSyncOperation, Vec<Vec<u8>>), String> {
    let mut operation = request.operation;
    if operation.direction != ConversationPlanDirection::Pull {
        return Err("provider materialization requires a Pull operation".to_string());
    }
    if operation.phase != ConversationSyncPhase::Prepared || !operation.targets.is_empty() {
        return Err("new conversation operation must begin Prepared without targets".to_string());
    }
    if request.targets.len() > 64 {
        return Err("conversation materialization has too many provider targets".to_string());
    }
    if operation.materialization_state == ConversationMaterializationState::Resumable
        && request.targets.is_empty()
        && !operation.is_pure_rekey()
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
        let staged_sha256 = sha256(&payload.bytes);
        payloads.push(payload.bytes);
        targets.push(ConversationJournalTarget {
            target_path: path_text(&target_path)?,
            role: payload.role,
            expected: payload.expected,
            staged_path: Some(path_text(&staged_path)?),
            backup_path: backup_path
                .as_ref()
                .map(|path| path_text(path))
                .transpose()?,
            staged_sha256: Some(staged_sha256),
            commit_started: false,
            committed: false,
        });
    }
    operation.targets = targets;
    operation.validate()?;
    Ok((operation, payloads))
}

/// The intent payload makes a Prepared operation restartable even if the
/// process exits before creating its provider-parent staging files. It stays
/// below Mallard's private root and is removed with the journal operation.
fn write_intent_payload(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
    index: usize,
    bytes: &[u8],
) -> Result<(), String> {
    let directory = intent_directory(repository, operation_id);
    create_private_directory_all(repository.root(), &directory)?;
    let path = directory.join(format!("{index}.payload"));
    write_new_file(&path, bytes)
}

fn intent_directory(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
) -> PathBuf {
    repository
        .root()
        .join("conversation_operations")
        .join(operation_id.as_str())
}

fn intent_payload_path(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
    index: usize,
) -> PathBuf {
    intent_directory(repository, operation_id).join(format!("{index}.payload"))
}

fn validate_operation_scope(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<(), String> {
    let identity = repository.load_or_create_host_identity()?;
    if identity.host_id != operation.host_id
        || identity.revision != operation.expected_host_identity_revision
    {
        return Err("conversation operation belongs to a different or changed host".to_string());
    }
    let machine = repository.load_bindings()?;
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
    let bindings = repository.load_conversation_bindings()?;
    if bindings.revision != operation.expected_bindings_document_revision {
        return Err("conversation bindings changed after review".to_string());
    }
    Ok(())
}

fn ensure_provider_target_parents(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
    targets: &[ConversationTargetPayload],
) -> Result<(), String> {
    if targets.is_empty() {
        return Ok(());
    }
    let machine = repository.load_bindings()?;
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

fn validate_target_expectations(targets: &[ConversationJournalTarget]) -> Result<(), String> {
    for target in targets {
        let path = Path::new(&target.target_path);
        match (&target.expected, read_regular_file_digest(path)?) {
            (TargetExpectation::Absent, None) => {}
            (TargetExpectation::Sha256(expected), Some(actual)) if expected == &actual => {}
            _ => return Err("conversation provider target changed after review".to_string()),
        }
    }
    Ok(())
}

fn insert_operation(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<(), String> {
    insert_operations(repository, std::slice::from_ref(operation))
}

fn insert_operations(
    repository: &LocalStateRepository,
    operations: &[ConversationSyncOperation],
) -> Result<(), String> {
    let journal = repository.load_conversation_sync_journal()?;
    repository.mutate_conversation_sync_journal(journal.revision, |journal| {
        for operation in operations {
            if journal
                .operations
                .iter()
                .any(|existing| existing.operation_id == operation.operation_id)
            {
                return Err(format!(
                    "conversation operation '{}' already exists",
                    operation.operation_id
                ));
            }
        }
        for operation in operations {
            journal.operations.push(operation.clone());
        }
        Ok(())
    })
}

fn mutate_operation<T>(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
    mutate: impl FnOnce(&mut ConversationSyncOperation) -> Result<T, String>,
) -> Result<(T, ConversationSyncOperation), String> {
    let journal = repository.load_conversation_sync_journal()?;
    let mut operation_after = None;
    let result = repository.mutate_conversation_sync_journal(journal.revision, |journal| {
        let operation = journal
            .operations
            .iter_mut()
            .find(|operation| &operation.operation_id == operation_id)
            .ok_or_else(|| format!("conversation operation '{}' disappeared", operation_id))?;
        let result = mutate(operation)?;
        operation_after = Some(operation.clone());
        Ok(result)
    })?;
    Ok((
        result,
        operation_after.expect("operation was found before mutation"),
    ))
}

fn set_phase(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
    phase: ConversationSyncPhase,
    updated_at: u64,
) -> Result<ConversationSyncOperation, String> {
    let (_, operation) = mutate_operation(repository, operation_id, |operation| {
        operation.phase = phase;
        operation.updated_at = operation.updated_at.max(updated_at);
        Ok(())
    })?;
    Ok(operation)
}

/// A reviewed batch pins one bindings-document revision for every action.
/// Earlier actions in the same journal batch legitimately advance that
/// document. Rebase only the global revision, and only while this action's
/// own binding pin still matches exactly.
fn rebase_operation_after_batch_progress(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<ConversationSyncOperation, String> {
    let document = repository.load_conversation_bindings()?;
    if document.revision == operation.expected_bindings_document_revision {
        return Ok(operation.clone());
    }
    let current_binding = document.bindings.iter().find(|binding| {
        binding.host_id == operation.host_id
            && binding.replica_id == operation.replica_id
            && binding.local_project_id == operation.local_project_id
            && binding.provider_profile_id == operation.provider_profile_id
            && binding.thread_id == operation.thread_id
    });
    match (operation.expected_binding_revision, current_binding) {
        (Some(expected), Some(binding)) if binding.revision == expected => {}
        (None, None) => {}
        _ if binding_commit_is_complete(&document, operation) => return Ok(operation.clone()),
        _ => return Err("conversation binding changed after review".to_string()),
    }
    let (_, updated) = mutate_operation(repository, &operation.operation_id, |journaled| {
        journaled.expected_bindings_document_revision = document.revision;
        Ok(())
    })?;
    Ok(updated)
}

fn stage_targets(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<(), String> {
    for (index, target) in operation.targets.iter().enumerate() {
        let point = ConversationFailpoint::BeforeStage {
            index,
            role: target.role.clone(),
        };
        failpoints.hit(&point)?;
        let intent = intent_payload_path(repository, &operation.operation_id, index);
        let expected = target
            .staged_sha256
            .as_deref()
            .ok_or_else(|| "conversation target lacks a staged digest".to_string())?;
        if read_regular_file_digest(&intent)?.as_deref() != Some(expected) {
            return Err("conversation intent payload failed verification".to_string());
        }
        verify_provider_payload(repository, operation, target, &intent)?;
        let staged = Path::new(
            target
                .staged_path
                .as_deref()
                .ok_or_else(|| "conversation target lacks a staging path".to_string())?,
        );
        ensure_canonical_parent(staged)?;
        match read_regular_file_digest(staged)? {
            Some(actual) if actual == expected => {}
            Some(_) => return Err("conversation staging path is already occupied".to_string()),
            None => copy_new_file(&intent, staged)?,
        }
        verify_provider_payload(repository, operation, target, staged)?;
        if let (TargetExpectation::Sha256(expected_target), Some(backup)) =
            (&target.expected, target.backup_path.as_deref())
        {
            let backup = Path::new(backup);
            create_private_directory_all(
                repository.root(),
                backup
                    .parent()
                    .ok_or_else(|| "conversation backup has no parent".to_string())?,
            )?;
            match read_regular_file_digest(backup)? {
                Some(actual) if &actual == expected_target => {}
                Some(_) => return Err("conversation backup digest mismatch".to_string()),
                None => copy_new_file(Path::new(&target.target_path), backup)?,
            }
        }
        sync_directory(
            staged
                .parent()
                .ok_or_else(|| "conversation stage has no parent".to_string())?,
        )?;
        failpoints.hit(&ConversationFailpoint::AfterStage {
            index,
            role: target.role.clone(),
        })?;
    }
    Ok(())
}

fn verify_target_role_set(
    operation: &ConversationSyncOperation,
    targets: &[ConversationTargetPayload],
) -> Result<(), String> {
    if operation.is_pure_rekey() {
        if !targets.is_empty() {
            return Err("conversation fork rekey must not touch provider targets".to_string());
        }
        return Ok(());
    }
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
    let creates_transcript = targets
        .iter()
        .find(|target| target.role == ConversationFileRole::Transcript)
        .is_some_and(|target| target.expected == TargetExpectation::Absent);
    if operation.provider == super::domain::Provider::Codex
        && ((creates_transcript && indexes != 1) || (!creates_transcript && indexes > 1))
    {
        return Err(
            "new Codex conversations require one index fragment; replacements allow at most one"
                .to_string(),
        );
    }
    // Claude has no provider index: the projects directory listing is the
    // index, so a resumable Claude materialization is exactly one transcript.
    if operation.provider == super::domain::Provider::Claude && indexes != 0 {
        return Err("Claude conversations have no provider index fragment".to_string());
    }
    Ok(())
}

fn verify_provider_payload(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
    target: &ConversationJournalTarget,
    path: &Path,
) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "read staged provider payload '{}': {}",
            path.display(),
            error
        )
    })?;
    match &target.role {
        ConversationFileRole::Transcript
            if operation.provider == super::domain::Provider::Claude =>
        {
            // Claude transcripts have no lineage codec. Verify what can be
            // proven without one: bounded JSONL structure, and that the
            // staged file's authoritative session metadata names exactly the
            // reviewed destination session in the reviewed project.
            if operation.lineage_digest.is_some() || operation.semantic_digest.is_some() {
                return Err(
                    "Claude conversations cannot pin lineage digests without a codec".to_string(),
                );
            }
            for line in bytes
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
            {
                if line.len() > 1024 * 1024 {
                    return Err("Claude transcript row exceeds one MiB".to_string());
                }
                serde_json::from_slice::<serde_json::Value>(line)
                    .map_err(|_| "Claude transcript contains malformed JSON".to_string())?;
            }
            let meta = super::provider_capture::scan_jsonl_metadata(
                path,
                super::provider_capture::Provider::Claude,
            )
            .map_err(|error| format!("staged Claude transcript metadata: {error}"))?;
            if Some(meta.session_id.as_str()) != operation.provider_thread_id.as_deref() {
                return Err("staged Claude transcript has the wrong session id".to_string());
            }
            let machine = repository.load_bindings()?;
            let project = machine
                .bindings
                .iter()
                .find(|binding| {
                    binding.host_id == operation.host_id
                        && binding.replica_id == operation.replica_id
                        && binding.local_project_id == operation.local_project_id
                })
                .ok_or_else(|| "staged transcript project replica disappeared".to_string())?;
            if meta.cwd != project.project_root && meta.cwd != project.canonical_project_root {
                return Err("staged Claude transcript has the wrong project cwd".to_string());
            }
        }
        ConversationFileRole::Transcript => {
            if operation.provider != super::domain::Provider::Codex {
                return Err("unsupported resumable conversation provider".to_string());
            }
            let analysis = analyze_codex_transcript(&bytes);
            if analysis.source_thread_id.as_deref() != operation.provider_thread_id.as_deref() {
                return Err("staged Codex transcript has the wrong provider thread id".to_string());
            }
            let machine = repository.load_bindings()?;
            let project = machine
                .bindings
                .iter()
                .find(|binding| {
                    binding.host_id == operation.host_id
                        && binding.replica_id == operation.replica_id
                        && binding.local_project_id == operation.local_project_id
                })
                .ok_or_else(|| "staged transcript project replica disappeared".to_string())?;
            if analysis.source_cwd.as_deref() != Some(project.project_root.as_str())
                && analysis.source_cwd.as_deref() != Some(project.canonical_project_root.as_str())
            {
                return Err("staged Codex transcript has the wrong project cwd".to_string());
            }
            match analysis.lineage {
                CodecLineage::Complete {
                    lineage_digest,
                    semantic_digest,
                    ..
                } if Some(&lineage_digest) == operation.lineage_digest.as_ref()
                    && Some(&semantic_digest) == operation.semantic_digest.as_ref() => {}
                _ => {
                    return Err(
                        "staged Codex transcript changed lineage or semantic content".to_string(),
                    )
                }
            }
        }
        ConversationFileRole::ProviderIndexFragment => {
            let thread_id = operation
                .provider_thread_id
                .as_deref()
                .ok_or_else(|| "provider index lacks a destination thread id".to_string())?;
            validate_codex_index_fragment(&bytes, thread_id)?;
        }
        ConversationFileRole::Sidecar { .. } => {
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|_| "conversation sidecar is not valid JSON".to_string())?;
        }
    }
    Ok(())
}

fn validate_codex_index_fragment(bytes: &[u8], thread_id: &str) -> Result<(), String> {
    let mut row_count = 0_usize;
    let mut destination_count = 0_usize;
    let mut ids = std::collections::BTreeSet::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        if line.len() > 1024 * 1024 {
            return Err("Codex index row exceeds one MiB".to_string());
        }
        let value: serde_json::Value = serde_json::from_slice(line)
            .map_err(|_| "Codex index fragment contains malformed JSON".to_string())?;
        let object = value
            .as_object()
            .ok_or_else(|| "Codex index fragment row is not an object".to_string())?;
        let id = object
            .get("id")
            .or_else(|| object.get("session_id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "Codex index fragment row lacks a thread id".to_string())?;
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
            return Err("Codex index fragment has an invalid thread id".to_string());
        }
        if !ids.insert(id.to_string()) {
            return Err("Codex index fragment repeats a thread id".to_string());
        }
        destination_count += usize::from(id == thread_id);
        row_count += 1;
    }
    if row_count == 0 || destination_count != 1 {
        return Err(
            "Codex index target must contain exactly one destination thread row".to_string(),
        );
    }
    Ok(())
}

fn commit_provider_targets(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<(), String> {
    let count = repository
        .load_conversation_sync_journal()?
        .operations
        .iter()
        .find(|operation| &operation.operation_id == operation_id)
        .ok_or_else(|| "conversation operation disappeared".to_string())?
        .targets
        .len();
    for index in 0..count {
        let operation = repository
            .load_conversation_sync_journal()?
            .operations
            .into_iter()
            .find(|operation| &operation.operation_id == operation_id)
            .ok_or_else(|| "conversation operation disappeared".to_string())?;
        let target = operation.targets[index].clone();
        failpoints.hit(&ConversationFailpoint::BeforeTargetCommit {
            index,
            role: target.role.clone(),
        })?;
        if !target.commit_started {
            mutate_operation(repository, operation_id, |operation| {
                operation.targets[index].commit_started = true;
                Ok(())
            })?;
        }
        commit_one_target(&target)?;
        failpoints.hit(&ConversationFailpoint::AfterTargetWrite {
            index,
            role: target.role.clone(),
        })?;
        mutate_operation(repository, operation_id, |operation| {
            operation.targets[index].commit_started = true;
            operation.targets[index].committed = true;
            Ok(())
        })?;
        if let Some(stage) = &target.staged_path {
            remove_file_if_exists(Path::new(stage))?;
        }
        failpoints.hit(&ConversationFailpoint::AfterTargetCommit {
            index,
            role: target.role,
        })?;
    }
    Ok(())
}

fn commit_one_target(target: &ConversationJournalTarget) -> Result<(), String> {
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
    let current = read_regular_file_digest(path)?;
    if target.committed {
        return if current.as_deref() == Some(staged_digest) {
            Ok(())
        } else {
            Err("committed conversation provider target changed".to_string())
        };
    }
    if target.commit_started && current.as_deref() == Some(staged_digest) {
        match &target.expected {
            TargetExpectation::Absent => {
                if stage.exists() && same_file(stage, path)? {
                    return Ok(());
                }
                return Err("cannot prove ownership of an occupied conversation target".to_string());
            }
            TargetExpectation::Sha256(expected) => {
                let backup = target
                    .backup_path
                    .as_deref()
                    .ok_or_else(|| "replacement lacks a recoverable backup".to_string())?;
                if read_regular_file_digest(Path::new(backup))?.as_deref() == Some(expected) {
                    return Ok(());
                }
                return Err("conversation replacement backup failed verification".to_string());
            }
        }
    }
    if read_regular_file_digest(stage)?.as_deref() != Some(staged_digest) {
        return Err("conversation staged provider target is unavailable".to_string());
    }
    match (&target.expected, current) {
        (TargetExpectation::Absent, None) => {
            fs::hard_link(stage, path).map_err(|error| {
                format!(
                    "publish no-clobber conversation target '{}': {}",
                    path.display(),
                    error
                )
            })?;
        }
        (TargetExpectation::Sha256(expected), Some(actual)) if expected == &actual => {
            fs::rename(stage, path).map_err(|error| {
                format!(
                    "replace conversation target '{}': {}",
                    path.display(),
                    error
                )
            })?;
        }
        _ => return Err("conversation provider target changed during apply".to_string()),
    }
    sync_directory(
        path.parent()
            .ok_or_else(|| "conversation target has no parent".to_string())?,
    )
}

fn commit_binding(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<(), String> {
    let current = repository.load_conversation_bindings()?;
    if binding_commit_is_complete(&current, operation) {
        return Ok(());
    }
    let scope = operation_scope(operation);
    repository.mutate_conversation_bindings(|document| {
        if document.revision != operation.expected_bindings_document_revision {
            return Err("conversation bindings changed after review".to_string());
        }
        if !operation.rekey_committed {
            if let Some(old) = operation.old_thread_id.as_ref() {
                rekey_thread_binding(
                    document,
                    &scope,
                    old,
                    &operation.result_thread_id,
                    operation.expected_bindings_document_revision,
                    operation
                        .expected_binding_revision
                        .ok_or_else(|| "conversation rekey lacks a binding revision".to_string())?,
                    operation.updated_at,
                )?;
            }
        }
        if operation.is_pure_rekey() {
            // A deferred-publication fork moves the binding and nothing else:
            // the transcript, provider files, and observed digests are
            // already true for the minted thread, and there is no cloud
            // publication to pin.
            return Ok(());
        }

        let binding_target_thread_id = operation.binding_target_thread_id();
        if let Some(binding) = document.bindings.iter_mut().find(|binding| {
            binding.host_id == operation.host_id
                && binding.replica_id == operation.replica_id
                && binding.local_project_id == operation.local_project_id
                && binding.provider_profile_id == operation.provider_profile_id
                && binding.thread_id == *binding_target_thread_id
        }) {
            if operation.old_thread_id.is_none()
                && operation.expected_binding_revision.is_some()
                && binding.revision != operation.expected_binding_revision.unwrap()
            {
                return Err("conversation binding changed after review".to_string());
            }
            binding.provider_thread_id = operation.provider_thread_id.clone();
            binding.forked_from = operation.binding_forked_from.clone();
            binding.materialization_state = operation.materialization_state;
            binding.last_observed_file_sha256 = transcript_target_digest(operation);
            binding.last_observed_lineage_digest = operation.lineage_digest.clone();
            binding.last_observed_semantic_digest = operation.semantic_digest.clone();
            binding.source_generation = Some(operation.generation);
            binding.source_manifest_sha256 = Some(operation.manifest_sha256.clone());
            binding.revision = binding.revision.saturating_add(1);
            binding.updated_at = operation.updated_at;
        } else {
            document.bindings.push(LocalConversationBinding {
                host_id: operation.host_id.clone(),
                replica_id: operation.replica_id.clone(),
                local_project_id: operation.local_project_id.clone(),
                provider_profile_id: operation.provider_profile_id.clone(),
                thread_id: binding_target_thread_id.clone(),
                forked_from: operation.binding_forked_from.clone(),
                provider_thread_id: operation.provider_thread_id.clone(),
                materialization_state: operation.materialization_state,
                last_observed_file_sha256: transcript_target_digest(operation),
                last_observed_head_message_id: None,
                last_observed_lineage_digest: operation.lineage_digest.clone(),
                last_observed_semantic_digest: operation.semantic_digest.clone(),
                source_generation: Some(operation.generation),
                source_manifest_sha256: Some(operation.manifest_sha256.clone()),
                revision: 0,
                created_at: operation.updated_at,
                updated_at: operation.updated_at,
            });
        }
        Ok(())
    })
}

fn operation_scope(operation: &ConversationSyncOperation) -> ConversationBindingScope {
    ConversationBindingScope {
        host_id: operation.host_id.clone(),
        replica_id: operation.replica_id.clone(),
        local_project_id: operation.local_project_id.clone(),
        provider_profile_id: operation.provider_profile_id.clone(),
    }
}

fn binding_commit_is_complete(
    document: &super::domain::ConversationBindings,
    operation: &ConversationSyncOperation,
) -> bool {
    let materialized_exists = document.bindings.iter().any(|binding| {
        let in_scope = binding.host_id == operation.host_id
            && binding.replica_id == operation.replica_id
            && binding.local_project_id == operation.local_project_id
            && binding.provider_profile_id == operation.provider_profile_id
            && binding.thread_id == *operation.binding_target_thread_id();
        if !in_scope {
            return false;
        }
        if operation.is_pure_rekey() {
            // The rekey clears the cloud pins; everything else is untouched.
            return binding.source_generation.is_none();
        }
        binding.provider_thread_id == operation.provider_thread_id
            && binding.forked_from == operation.binding_forked_from
            && binding.materialization_state == operation.materialization_state
            && binding.last_observed_file_sha256 == transcript_target_digest(operation)
            && binding.last_observed_lineage_digest == operation.lineage_digest
            && binding.last_observed_semantic_digest == operation.semantic_digest
            && binding.source_generation == Some(operation.generation)
            && binding.source_manifest_sha256.as_deref() == Some(&operation.manifest_sha256)
    });
    let rekey_complete = operation.old_thread_id.as_ref().is_none_or(|old| {
        document.bindings.iter().any(|binding| {
            binding.host_id == operation.host_id
                && binding.replica_id == operation.replica_id
                && binding.local_project_id == operation.local_project_id
                && binding.provider_profile_id == operation.provider_profile_id
                && binding.thread_id == operation.result_thread_id
                && binding.forked_from.as_ref() == Some(old)
        })
    });
    materialized_exists && rekey_complete
}

fn transcript_target_digest(operation: &ConversationSyncOperation) -> Option<String> {
    operation
        .targets
        .iter()
        .find(|target| target.role == ConversationFileRole::Transcript)
        .and_then(|target| target.staged_sha256.clone())
        .or_else(|| operation.expected_local_digest.clone())
}

fn commit_rekey(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<ConversationSyncOperation, String> {
    let Some(old) = operation.old_thread_id.as_ref() else {
        return Ok(operation.clone());
    };
    if operation.rekey_committed {
        return Ok(operation.clone());
    }
    let scope = operation_scope(operation);
    let current = repository.load_conversation_bindings()?;
    let rekey_already_persisted = current.bindings.iter().any(|binding| {
        binding.host_id == operation.host_id
            && binding.replica_id == operation.replica_id
            && binding.local_project_id == operation.local_project_id
            && binding.provider_profile_id == operation.provider_profile_id
            && binding.thread_id == operation.result_thread_id
            && binding.forked_from.as_ref() == Some(old)
    });
    if !rekey_already_persisted {
        repository.mutate_conversation_bindings(|document| {
            rekey_thread_binding(
                document,
                &scope,
                old,
                &operation.result_thread_id,
                operation.expected_bindings_document_revision,
                operation
                    .expected_binding_revision
                    .ok_or_else(|| "conversation rekey lacks a binding revision".to_string())?,
                operation.updated_at,
            )?;
            Ok(())
        })?;
    }
    let bindings = repository.load_conversation_bindings()?;
    let rekeyed_binding = bindings
        .bindings
        .iter()
        .find(|binding| {
            binding.host_id == operation.host_id
                && binding.replica_id == operation.replica_id
                && binding.local_project_id == operation.local_project_id
                && binding.provider_profile_id == operation.provider_profile_id
                && binding.thread_id == operation.result_thread_id
        })
        .filter(|binding| binding.forked_from.as_ref() == Some(old))
        .ok_or_else(|| "conversation rekey was not persisted".to_string())?;
    let (_, updated) = mutate_operation(repository, &operation.operation_id, |journaled| {
        journaled.rekey_committed = true;
        journaled.expected_bindings_document_revision = bindings.revision;
        journaled.expected_binding_revision = Some(rekeyed_binding.revision);
        Ok(())
    })?;
    Ok(updated)
}

fn commit_receipt(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<ConversationApplyReceipt, String> {
    let receipt = conversation_receipt(operation);
    let plan_id = operation
        .plan_id
        .clone()
        .ok_or_else(|| "conversation materialization lacks a plan id".to_string())?;
    repository.mutate_materializations(|_, materializations| {
        if materializations.records.iter().any(|record| {
            record.receipts.iter().any(|receipt| {
                receipt
                    .conversation
                    .as_ref()
                    .is_some_and(|existing| existing.journal_operation_id == operation.operation_id)
            })
        }) {
            return Ok(());
        }
        let action_id = operation
            .plan_action_id
            .clone()
            .map(Ok)
            .unwrap_or_else(|| stable_action_id(&operation.operation_id))?;
        let resource_id =
            conversation_resource_id(operation.provider, operation.binding_target_thread_id())?;
        let apply_receipt = ApplyReceipt {
            action_id,
            resource_id,
            action_type: RestoreActionType::MaterializeConversation,
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
        if let Some(record) = materializations
            .records
            .iter_mut()
            .find(|record| record.plan_id == plan_id)
        {
            if record.host_id != operation.host_id
                || record.replica_id != operation.replica_id
                || record.local_project_id != operation.local_project_id
                || record.storage_id != operation.storage_id
                || record.bundle_id != operation.bundle_id
                || record.generation != operation.generation
                || record.commit_id != operation.commit_id
                || record.manifest_sha256 != operation.manifest_sha256
            {
                return Err(
                    "conversation receipt plan scope disagrees with materialization".to_string(),
                );
            }
            record.receipts.push(apply_receipt);
            record.applied_at = record.applied_at.max(operation.updated_at);
        } else {
            materializations.records.push(MaterializationRecord {
                materialization_id: stable_materialization_id(&operation.operation_id)?,
                plan_id,
                host_id: operation.host_id.clone(),
                replica_id: operation.replica_id.clone(),
                local_project_id: operation.local_project_id.clone(),
                storage_id: operation.storage_id.clone(),
                bundle_id: operation.bundle_id.clone(),
                generation: operation.generation,
                commit_id: operation.commit_id.clone(),
                manifest_sha256: operation.manifest_sha256.clone(),
                binding_revision: operation.expected_project_binding_revision,
                status: MaterializationStatus::Complete,
                applied_at: operation.updated_at,
                receipts: vec![apply_receipt],
            });
        }
        materializations.schema = LOCAL_SCHEMA;
        Ok(())
    })?;
    Ok(receipt)
}

fn conversation_receipt(operation: &ConversationSyncOperation) -> ConversationApplyReceipt {
    ConversationApplyReceipt {
        host_id: operation.host_id.clone(),
        replica_id: operation.replica_id.clone(),
        old_thread_id: operation.old_thread_id.clone(),
        result_thread_id: operation.result_thread_id.clone(),
        provider_profile_id: operation.provider_profile_id.clone(),
        provider_thread_id: operation.provider_thread_id.clone(),
        materialization_state: operation.materialization_state,
        lineage_digest: operation.lineage_digest.clone(),
        semantic_digest: operation.semantic_digest.clone(),
        local_file_sha256_after: transcript_target_digest(operation),
        cloud_generation: operation.generation,
        cloud_commit_id: operation.commit_id.clone(),
        cloud_manifest_sha256: operation.manifest_sha256.clone(),
        published_origin_host_token: None,
        published_origin_replica_token: None,
        journal_operation_id: operation.operation_id.clone(),
    }
}

fn cleanup_and_retire(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
) -> Result<(), String> {
    let operation = repository
        .load_conversation_sync_journal()?
        .operations
        .into_iter()
        .find(|operation| &operation.operation_id == operation_id)
        .ok_or_else(|| format!("conversation operation '{}' disappeared", operation_id))?;
    for target in &operation.targets {
        if let Some(stage) = &target.staged_path {
            remove_file_if_exists(Path::new(stage))?;
        }
    }
    remove_directory_tree_if_owned(
        &intent_directory(repository, operation_id),
        repository.root(),
    )?;
    let journal = repository.load_conversation_sync_journal()?;
    repository.mutate_conversation_sync_journal(journal.revision, |journal| {
        journal
            .operations
            .retain(|operation| &operation.operation_id != operation_id);
        Ok(())
    })
}

fn rollback_prepared(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<(), String> {
    for target in &operation.targets {
        if let Some(stage) = &target.staged_path {
            remove_file_if_exists(Path::new(stage))?;
        }
    }
    cleanup_and_retire(repository, &operation.operation_id)
}

fn mark_recovery_required(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
) -> Result<(), String> {
    set_phase(
        repository,
        operation_id,
        ConversationSyncPhase::RecoveryRequired,
        0,
    )
    .map(|_| ())
}

fn staged_target_is_valid(target: &ConversationJournalTarget) -> bool {
    target
        .staged_path
        .as_deref()
        .zip(target.staged_sha256.as_deref())
        .and_then(|(path, digest)| {
            read_regular_file_digest(Path::new(path))
                .ok()
                .flatten()
                .map(|actual| (actual, digest))
        })
        .is_some_and(|(actual, digest)| actual == digest)
}

fn target_is_uncommitted(target: &ConversationJournalTarget) -> bool {
    if target.commit_started || target.committed {
        return false;
    }
    match (
        &target.expected,
        read_regular_file_digest(Path::new(&target.target_path)),
    ) {
        (TargetExpectation::Absent, Ok(None)) => true,
        (TargetExpectation::Sha256(expected), Ok(Some(actual))) => expected == &actual,
        _ => false,
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

fn read_regular_file_digest(path: &Path) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
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
    Ok(Some(hex_digest(hasher.finalize().as_slice())))
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

fn remove_directory_tree_if_owned(path: &Path, root: &Path) -> Result<(), String> {
    if !path.starts_with(root) {
        return Err("conversation cleanup escapes Mallard state".to_string());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)
                .map_err(|error| format!("remove '{}': {}", path.display(), error))?;
            sync_directory(
                path.parent()
                    .ok_or_else(|| "conversation cleanup has no parent".to_string())?,
            )
        }
        Ok(_) => Err(format!("'{}' is not a real directory", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect '{}': {}", path.display(), error)),
    }
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(path)
                .map_err(|error| format!("remove '{}': {}", path.display(), error))?;
            sync_directory(
                path.parent()
                    .ok_or_else(|| "removed file has no parent".to_string())?,
            )
        }
        Ok(_) => Err(format!(
            "'{}' is not a regular no-follow file",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect '{}': {}", path.display(), error)),
    }
}

fn same_file(left: &Path, right: &Path) -> Result<bool, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let left = fs::metadata(left)
            .map_err(|error| format!("inspect '{}': {}", left.display(), error))?;
        let right = fs::metadata(right)
            .map_err(|error| format!("inspect '{}': {}", right.display(), error))?;
        Ok(left.dev() == right.dev() && left.ino() == right.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        Ok(false)
    }
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
    sha256(value.as_bytes())[..24].to_string()
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

fn sha256(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes).as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
