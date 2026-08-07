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

use crate::codex_native_fork::{
    complete_codex_post_commit_presentation, coordinate_native_fork, preflight_native_fork,
    verify_recorded_fork, verify_source_idle_and_title, CodexForkCoordinatorOutcome,
    CodexForkExecutionRequest, CodexNameStatus, CodexPostCommitPresentationRequest,
    CodexThreadRenameRequest, ForkPhaseObserver, VerifiedNativeFork,
};
use crate::conversation_codec::{analyze_codex_transcript, CodecLineage};
use crate::persistence::LocalStateRepository;
use mallard_domain::{
    conversation_resource_id, ActionId, ActionStatus, ApplyReceipt, CodexDivergenceReceipt,
    CodexForkAttempt, CodexForkAttemptStatus, CodexForkExecutor, CodexInventoryRefreshStatus,
    ConversationApplyReceipt, ConversationCloudAnchor, ConversationDivergenceResolution,
    ConversationFileRole, ConversationJournalTarget, ConversationMaterializationState,
    ConversationNameStatus, ConversationOperationId, ConversationPlanDirection,
    ConversationSyncOperation, ConversationSyncOperationKind, ConversationSyncPhase, ForkOrigin,
    ForkStamp, LocalConversationBinding, MaterializationId, MaterializationRecord,
    MaterializationStatus, RestoreActionType, TargetExpectation, ThreadId, LOCAL_SCHEMA,
};

const MAX_PROVIDER_TARGET_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug)]
struct CodexPostCommitPresentationTask {
    operation_id: ConversationOperationId,
    request: CodexPostCommitPresentationRequest,
    /// Pull operations record the deferred naming outcome into their
    /// materialization receipt. A manual fork has no receipt to update; its
    /// task still performs the rename and inventory refresh.
    records_receipt: bool,
}

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

#[derive(Clone, Debug)]
pub struct CodexDivergenceMaterializationRequest {
    pub operation: ConversationSyncOperation,
    pub targets: Vec<ConversationTargetPayload>,
    pub fork: CodexForkExecutionRequest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodexDivergenceApplyOutcome {
    Committed(ConversationApplyReceipt),
    /// A manual-origin fork adopted and stamped; no cloud state, binding pin,
    /// or materialization receipt exists to report.
    ManualForkCommitted {
        operation_id: ConversationOperationId,
        preserved_thread_id: ThreadId,
        preserved_provider_thread_id: String,
        desired_fork_name: String,
        executor: Option<CodexForkExecutor>,
    },
    ForkUnacknowledged {
        operation_id: ConversationOperationId,
        message: String,
    },
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
    AfterForkVerified,
    AfterForkNamingRecorded,
    AfterLocalForkBound,
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
    AfterBindingsCommitted,
    BeforeReceipt,
    AfterReceipt,
    AfterOriginalNamingRecorded,
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
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
}

/// Journal a reviewed Push, publish through the caller's production bundle
/// engine, and finalize the local receipt. `inspect_after_error` closes the
/// important CAS ambiguity: an object store may commit the head and then
/// return an I/O error.
#[cfg(any(test, feature = "test-support"))]
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
        || operation.operation != ConversationSyncOperationKind::CloudPublication
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
        record_publication_anchor(operation, &publication)?;
        operation.phase = ConversationSyncPhase::CloudPublished;
        Ok(())
    })?;
    operation = updated;
    failpoints.hit(&ConversationFailpoint::AfterCloudPublished)?;

    // Preserve the existing binding until immutable publication and the head
    // CAS are proven complete. Recovery can safely finish this exact binding
    // refresh after a post-CAS crash.
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
pub fn apply_conversation_cloud_publications_locked(
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
            || operation.operation != ConversationSyncOperationKind::CloudPublication
            || !operation.targets.is_empty()
        {
            return Err("cloud publication requires Prepared Push operations".to_string());
        }
        operation.validate()?;
        validate_operation_scope(repository, operation)?;
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
            record_publication_anchor(operation, &publication)?;
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
#[cfg(any(test, feature = "test-support"))]
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
                        record_publication_anchor(operation, &publication)?;
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
                    record_publication_anchor(operation, &publication)?;
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
#[cfg(any(test, feature = "test-support"))]
pub fn apply_conversation_materialization(
    repository: &LocalStateRepository,
    request: ConversationMaterializationRequest,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<ConversationApplyReceipt, String> {
    let _host_guard = repository.acquire_host_operation()?;
    apply_conversation_materialization_locked(repository, request, failpoints)
}

pub fn apply_conversation_materialization_locked(
    repository: &LocalStateRepository,
    request: ConversationMaterializationRequest,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<ConversationApplyReceipt, String> {
    request.operation.validate()?;
    if request.operation.operation != ConversationSyncOperationKind::ProviderMaterialization {
        return Err("generic materialization cannot execute a Codex divergence".to_string());
    }
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
        ConversationSyncPhase::ReplacementStaged,
        operation.updated_at,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterStaged)?;

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

pub fn apply_codex_divergence_locked(
    repository: &LocalStateRepository,
    request: CodexDivergenceMaterializationRequest,
    failpoints: &mut impl ConversationFailpoints,
    on_phase: ForkPhaseObserver<'_>,
) -> Result<CodexDivergenceApplyOutcome, String> {
    request.operation.validate()?;
    if !matches!(
        request.operation.operation,
        ConversationSyncOperationKind::CodexDivergence(_)
    ) {
        return Err("Codex divergence apply requires a divergence journal".to_string());
    }
    validate_operation_scope(repository, &request.operation)?;
    ensure_provider_target_parents(repository, &request.operation, &request.targets)?;
    validate_fork_request(&request.operation, &request.fork)?;
    if request.operation.divergence().is_some_and(|divergence| {
        divergence.resolution == ConversationDivergenceResolution::PreserveLocalWithNativeFork
    }) {
        preflight_native_fork(&request.fork, on_phase)?;
    }
    let (mut operation, payloads) = prepare_operation(
        repository,
        ConversationMaterializationRequest {
            operation: request.operation,
            targets: request.targets,
        },
    )?;
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
        ConversationSyncPhase::ReplacementStaged,
        operation.updated_at,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterStaged)?;

    let resolution = operation
        .divergence()
        .ok_or_else(|| "Codex divergence journal disappeared".to_string())?
        .resolution;
    if resolution == ConversationDivergenceResolution::PreserveLocalWithNativeFork {
        match continue_fork_attempts(repository, operation, &request.fork, on_phase)? {
            ForkAttemptContinuation::Verified(updated) => operation = updated,
            ForkAttemptContinuation::AbortedBeforeFork { operation, message } => {
                rollback_prepared(repository, &operation)?;
                return Err(message);
            }
            ForkAttemptContinuation::Unacknowledged { operation, message } => {
                return Ok(CodexDivergenceApplyOutcome::ForkUnacknowledged {
                    operation_id: operation.operation_id,
                    message,
                });
            }
        }
        failpoints.hit(&ConversationFailpoint::AfterForkVerified)?;
        return finish_verified_codex_divergence(repository, operation, &request.fork, failpoints);
    } else {
        if let Err(error) = verify_source_idle_and_title(&request.fork) {
            rollback_prepared(repository, &operation)?;
            return Err(error);
        }
    }

    finish_codex_divergence(repository, operation, &request.fork, failpoints)
}

pub fn retry_codex_divergence_locked(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
    mut fork: CodexForkExecutionRequest,
    failpoints: &mut impl ConversationFailpoints,
    on_phase: ForkPhaseObserver<'_>,
) -> Result<CodexDivergenceApplyOutcome, String> {
    let mut operation = load_operation(repository, operation_id)?;
    validate_operation_scope_for_resume(repository, &operation)?;
    validate_fork_request(&operation, &fork)?;
    let divergence = operation
        .divergence()
        .ok_or_else(|| "conversation operation is not a Codex divergence".to_string())?;
    if divergence.resolution != ConversationDivergenceResolution::PreserveLocalWithNativeFork
        || operation.phase != ConversationSyncPhase::ForkUnacknowledged
    {
        return Err("only an unacknowledged Codex fork can be retried".to_string());
    }
    let executor = divergence
        .attempts
        .last()
        .filter(|attempt| attempt.status == CodexForkAttemptStatus::Unacknowledged)
        .map(|attempt| attempt.executor)
        .ok_or_else(|| "Codex fork retry lacks an unacknowledged attempt".to_string())?;
    let retry_start = divergence
        .available_executors
        .iter()
        .position(|available| *available == executor)
        .ok_or_else(|| "Codex fork retry executor is no longer available".to_string())?;
    let prior_attempts = divergence.attempts.clone();
    // The user-authorized retry starts again at the unacknowledged executor.
    // A definitive no-fork result may then use a later executor, while another
    // unacknowledged result still stops the sequence immediately.
    fork.available_executors = divergence.available_executors[retry_start..].to_vec();
    match coordinate_native_fork(
        &fork,
        |executor, status| record_fork_attempt(repository, operation_id, executor, status),
        on_phase,
    )? {
        CodexForkCoordinatorOutcome::Verified(verified) => {
            operation = record_verified_fork(repository, &operation, &verified)?;
        }
        CodexForkCoordinatorOutcome::Unacknowledged { message, .. } => {
            operation = load_operation(repository, operation_id)?;
            return Ok(CodexDivergenceApplyOutcome::ForkUnacknowledged {
                operation_id: operation.operation_id,
                message,
            });
        }
        CodexForkCoordinatorOutcome::AbortedBeforeInvocation { message } => {
            restore_unacknowledged_retry_checkpoint(
                repository,
                operation_id,
                executor,
                prior_attempts,
            )?;
            return Ok(CodexDivergenceApplyOutcome::ForkUnacknowledged {
                operation_id: operation_id.clone(),
                message: format!("The fork retry was not invoked: {message}"),
            });
        }
    }
    failpoints.hit(&ConversationFailpoint::AfterForkVerified)?;
    finish_verified_codex_divergence(repository, operation, &fork, failpoints)
}

fn finish_verified_codex_divergence(
    repository: &LocalStateRepository,
    mut operation: ConversationSyncOperation,
    fork: &CodexForkExecutionRequest,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<CodexDivergenceApplyOutcome, String> {
    if operation.phase == ConversationSyncPhase::ForkVerified {
        operation = set_phase(
            repository,
            &operation.operation_id,
            ConversationSyncPhase::ForkNamingRecorded,
            operation.updated_at,
        )?;
        failpoints.hit(&ConversationFailpoint::AfterForkNamingRecorded)?;
    }
    if operation.phase == ConversationSyncPhase::ForkNamingRecorded {
        verify_journaled_fork(&operation, fork)?;
        operation = commit_preserved_fork_binding(repository, &operation)?;
        operation = set_phase(
            repository,
            &operation.operation_id,
            ConversationSyncPhase::LocalForkBound,
            operation.updated_at,
        )?;
        failpoints.hit(&ConversationFailpoint::AfterLocalForkBound)?;
    }
    finish_codex_divergence(repository, operation, fork, failpoints)
}

enum ForkAttemptContinuation {
    Verified(ConversationSyncOperation),
    AbortedBeforeFork {
        operation: ConversationSyncOperation,
        message: String,
    },
    Unacknowledged {
        operation: ConversationSyncOperation,
        message: String,
    },
}

fn continue_fork_attempts(
    repository: &LocalStateRepository,
    operation: ConversationSyncOperation,
    fork: &CodexForkExecutionRequest,
    on_phase: ForkPhaseObserver<'_>,
) -> Result<ForkAttemptContinuation, String> {
    let divergence = operation
        .divergence()
        .ok_or_else(|| "conversation operation is not a Codex divergence".to_string())?;
    if let Some(attempt) = divergence.attempts.last() {
        if attempt.status == CodexForkAttemptStatus::Pending {
            record_fork_attempt(
                repository,
                &operation.operation_id,
                attempt.executor,
                CodexForkAttemptStatus::Unacknowledged,
            )?;
            let operation = load_operation(repository, &operation.operation_id)?;
            return Ok(ForkAttemptContinuation::Unacknowledged {
                operation,
                message:
                    "Mallard restarted before acknowledging the Codex fork; the result was ignored"
                        .to_string(),
            });
        }
        if attempt.status == CodexForkAttemptStatus::Unacknowledged {
            return Ok(ForkAttemptContinuation::Unacknowledged {
                operation,
                message:
                    "Mallard did not acknowledge the Codex fork; retrying may create another fork"
                        .to_string(),
            });
        }
        if attempt.status != CodexForkAttemptStatus::DefinitiveNoFork {
            return Err("Codex fork attempt journal cannot be continued safely".to_string());
        }
    }

    let completed_attempts = divergence.attempts.len();
    let remaining_executors = divergence
        .available_executors
        .get(completed_attempts..)
        .ok_or_else(|| "Codex fork attempt journal exceeds its executor list".to_string())?
        .to_vec();
    if remaining_executors.is_empty() {
        return Err("Codex fork attempt journal has no remaining executor".to_string());
    }
    let mut remaining_request = fork.clone();
    remaining_request.available_executors = remaining_executors;
    let operation_id = operation.operation_id.clone();
    match coordinate_native_fork(
        &remaining_request,
        |executor, status| record_fork_attempt(repository, &operation_id, executor, status),
        on_phase,
    )? {
        CodexForkCoordinatorOutcome::Verified(verified) => {
            record_verified_fork(repository, &operation, &verified)
                .map(ForkAttemptContinuation::Verified)
        }
        CodexForkCoordinatorOutcome::AbortedBeforeInvocation { message } => {
            let operation = load_operation(repository, &operation_id)?;
            Ok(ForkAttemptContinuation::AbortedBeforeFork { operation, message })
        }
        CodexForkCoordinatorOutcome::Unacknowledged { message, .. } => {
            let operation = load_operation(repository, &operation_id)?;
            Ok(ForkAttemptContinuation::Unacknowledged { operation, message })
        }
    }
}

fn finish_codex_divergence(
    repository: &LocalStateRepository,
    mut operation: ConversationSyncOperation,
    fork: &CodexForkExecutionRequest,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<CodexDivergenceApplyOutcome, String> {
    let origin = operation.fork_origin();
    if origin == Some(ForkOrigin::Manual)
        && operation.phase == ConversationSyncPhase::LocalForkBound
    {
        operation = record_post_commit_presentation(repository, &operation)?;
        failpoints.hit(&ConversationFailpoint::AfterOriginalNamingRecorded)?;
    }
    let may_commit_provider = origin != Some(ForkOrigin::Manual)
        && (operation.phase == ConversationSyncPhase::LocalForkBound
            || (operation.phase == ConversationSyncPhase::ReplacementStaged
                && operation.divergence().is_some_and(|divergence| {
                    divergence.resolution == ConversationDivergenceResolution::UseCloudOnly
                })));
    if may_commit_provider {
        commit_provider_targets(repository, &operation.operation_id, failpoints)?;
        failpoints.hit(&ConversationFailpoint::BeforeProviderCommitted)?;
        operation = set_phase(
            repository,
            &operation.operation_id,
            ConversationSyncPhase::ProviderCommitted,
            operation.updated_at,
        )?;
        failpoints.hit(&ConversationFailpoint::AfterProviderCommitted)?;
    }

    if operation.phase == ConversationSyncPhase::ProviderCommitted {
        failpoints.hit(&ConversationFailpoint::BeforeBindingCommit)?;
        commit_binding(repository, &operation)?;
        failpoints.hit(&ConversationFailpoint::AfterBindingCommit)?;
        operation = set_phase(
            repository,
            &operation.operation_id,
            ConversationSyncPhase::BindingsCommitted,
            operation.updated_at,
        )?;
        failpoints.hit(&ConversationFailpoint::AfterBindingsCommitted)?;
    }

    if operation.phase == ConversationSyncPhase::BindingsCommitted {
        operation = record_post_commit_presentation(repository, &operation)?;
        failpoints.hit(&ConversationFailpoint::AfterOriginalNamingRecorded)?;
    }

    if operation.phase == ConversationSyncPhase::OriginalNamingRecorded {
        let presentation_task = post_commit_presentation_task(&operation, fork)?;
        let outcome = if origin == Some(ForkOrigin::Manual) {
            manual_fork_outcome(&operation)?
        } else {
            failpoints.hit(&ConversationFailpoint::BeforeReceipt)?;
            let receipt = commit_receipt(repository, &operation)?;
            failpoints.hit(&ConversationFailpoint::AfterReceipt)?;
            CodexDivergenceApplyOutcome::Committed(receipt)
        };
        failpoints.hit(&ConversationFailpoint::BeforeLocalCommitted)?;
        operation = set_phase(
            repository,
            &operation.operation_id,
            ConversationSyncPhase::LocalCommitted,
            operation.updated_at,
        )?;
        failpoints.hit(&ConversationFailpoint::AfterLocalCommitted)?;
        failpoints.hit(&ConversationFailpoint::BeforeCleanup)?;
        cleanup_and_retire(repository, &operation.operation_id)?;
        failpoints.hit(&ConversationFailpoint::AfterCleanup)?;
        if let Some(task) = presentation_task {
            dispatch_post_commit_presentation(repository.clone(), task);
        }
        return Ok(outcome);
    }
    if operation.phase == ConversationSyncPhase::LocalCommitted {
        let outcome = if origin == Some(ForkOrigin::Manual) {
            manual_fork_outcome(&operation)?
        } else {
            CodexDivergenceApplyOutcome::Committed(conversation_receipt(
                &operation,
                operation.cloud_anchor()?,
            ))
        };
        let presentation_task = post_commit_presentation_task(&operation, fork)?;
        cleanup_and_retire(repository, &operation.operation_id)?;
        if let Some(task) = presentation_task {
            dispatch_post_commit_presentation(repository.clone(), task);
        }
        return Ok(outcome);
    }
    return Err(format!(
        "Codex divergence cannot continue safely from phase '{:?}'",
        operation.phase
    ));
}

fn manual_fork_outcome(
    operation: &ConversationSyncOperation,
) -> Result<CodexDivergenceApplyOutcome, String> {
    let divergence = operation
        .divergence()
        .ok_or_else(|| "conversation operation is not a Codex divergence".to_string())?;
    let preserved_provider_thread_id = divergence
        .fork_provider_thread_id
        .clone()
        .ok_or_else(|| "manual fork outcome lacks a verified fork".to_string())?;
    Ok(CodexDivergenceApplyOutcome::ManualForkCommitted {
        operation_id: operation.operation_id.clone(),
        preserved_thread_id: divergence.preserved_thread_id.clone(),
        preserved_provider_thread_id,
        desired_fork_name: divergence.desired_fork_name.clone(),
        executor: divergence.successful_executor,
    })
}

fn validate_fork_request(
    operation: &ConversationSyncOperation,
    fork: &CodexForkExecutionRequest,
) -> Result<(), String> {
    let divergence = operation
        .divergence()
        .ok_or_else(|| "conversation operation is not a Codex divergence".to_string())?;
    if operation.provider_thread_id.as_deref() != Some(&divergence.source_provider_thread_id)
        || fork.source_provider_thread_id != divergence.source_provider_thread_id
        || fork.source_transcript_path != PathBuf::from(&divergence.source_transcript_path)
        || fork.source_transcript_sha256 != operation.expected_local_digest.as_deref().unwrap_or("")
        || fork.local_lineage_digest != divergence.local_lineage_digest
        || fork.local_semantic_digest != divergence.local_semantic_digest
        || fork.pinned_local_title != divergence.pinned_local_title
        || fork.desired_fork_name != divergence.desired_fork_name
        || fork.codex_executable.to_string_lossy() != divergence.codex_executable
        || fork.codex_executable_sha256 != divergence.codex_executable_sha256
        || fork.available_executors != divergence.available_executors
    {
        return Err("Codex fork executor inputs differ from the reviewed journal".to_string());
    }
    Ok(())
}

fn load_operation(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
) -> Result<ConversationSyncOperation, String> {
    repository
        .load_conversation_sync_journal()?
        .operations
        .into_iter()
        .find(|operation| &operation.operation_id == operation_id)
        .ok_or_else(|| format!("conversation operation '{}' does not exist", operation_id))
}

fn validate_operation_scope_for_resume(
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
    let binding = machine
        .bindings
        .iter()
        .find(|binding| {
            binding.host_id == operation.host_id
                && binding.replica_id == operation.replica_id
                && binding.local_project_id == operation.local_project_id
        })
        .ok_or_else(|| "conversation operation references a missing project replica".to_string())?;
    if binding.revision != operation.expected_project_binding_revision
        || binding.profile_ids.get(&operation.provider) != Some(&operation.provider_profile_id)
    {
        return Err("conversation project binding changed after review".to_string());
    }
    let profile = machine
        .profiles
        .iter()
        .find(|profile| profile.profile_id == operation.provider_profile_id)
        .ok_or_else(|| "conversation provider profile disappeared".to_string())?;
    if profile.revision != operation.expected_profile_revision {
        return Err("conversation provider profile changed after review".to_string());
    }
    Ok(())
}

pub fn codex_fork_request_for_operation(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<CodexForkExecutionRequest, String> {
    operation.validate()?;
    let divergence = operation
        .divergence()
        .ok_or_else(|| "conversation operation is not a Codex divergence".to_string())?;
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
    let profile = machine
        .profiles
        .iter()
        .find(|profile| profile.profile_id == operation.provider_profile_id)
        .ok_or_else(|| {
            "conversation operation references a missing provider profile".to_string()
        })?;
    Ok(CodexForkExecutionRequest {
        codex_home: PathBuf::from(&profile.canonical_path),
        project_directory: PathBuf::from(&project_binding.project_root),
        codex_executable: PathBuf::from(&divergence.codex_executable),
        codex_executable_sha256: divergence.codex_executable_sha256.clone(),
        source_provider_thread_id: divergence.source_provider_thread_id.clone(),
        source_transcript_path: PathBuf::from(&divergence.source_transcript_path),
        source_transcript_sha256: operation
            .expected_local_digest
            .clone()
            .ok_or_else(|| "Codex divergence lacks its source transcript pin".to_string())?,
        local_lineage_digest: divergence.local_lineage_digest.clone(),
        local_semantic_digest: divergence.local_semantic_digest.clone(),
        pinned_local_title: divergence.pinned_local_title.clone(),
        desired_fork_name: divergence.desired_fork_name.clone(),
        available_executors: divergence.available_executors.clone(),
    })
}

fn divergence_mut(
    operation: &mut ConversationSyncOperation,
) -> Result<&mut mallard_domain::CodexDivergenceOperation, String> {
    match &mut operation.operation {
        ConversationSyncOperationKind::CodexDivergence(divergence) => Ok(divergence),
        _ => Err("conversation operation is not a Codex divergence".to_string()),
    }
}

fn record_fork_attempt(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
    executor: CodexForkExecutor,
    status: CodexForkAttemptStatus,
) -> Result<(), String> {
    // A verified executor result is not durable until its exact provider ID,
    // path, digest, and naming outcome can be committed in one mutation. If
    // Mallard stops in that narrow window, startup marks Pending as
    // Unacknowledged and deliberately does not discover or adopt a child.
    if status == CodexForkAttemptStatus::Verified {
        return Ok(());
    }
    mutate_operation(repository, operation_id, |journaled| {
        let divergence = divergence_mut(journaled)?;
        if let Some(attempt) = divergence
            .attempts
            .iter_mut()
            .find(|attempt| attempt.executor == executor)
        {
            attempt.status = status;
        } else {
            divergence
                .attempts
                .push(CodexForkAttempt { executor, status });
        }
        divergence.current_executor = matches!(
            status,
            CodexForkAttemptStatus::Pending | CodexForkAttemptStatus::Unacknowledged
        )
        .then_some(executor);
        match status {
            CodexForkAttemptStatus::Pending => {
                journaled.phase = ConversationSyncPhase::ForkAttemptPending
            }
            CodexForkAttemptStatus::Unacknowledged => {
                journaled.phase = ConversationSyncPhase::ForkUnacknowledged
            }
            CodexForkAttemptStatus::DefinitiveNoFork | CodexForkAttemptStatus::Verified => {}
        }
        Ok(())
    })
    .map(|_| ())
}

fn restore_unacknowledged_retry_checkpoint(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
    executor: CodexForkExecutor,
    prior_attempts: Vec<CodexForkAttempt>,
) -> Result<(), String> {
    mutate_operation(repository, operation_id, |journaled| {
        let divergence = divergence_mut(journaled)?;
        divergence.attempts = prior_attempts.clone();
        divergence.current_executor = Some(executor);
        journaled.phase = ConversationSyncPhase::ForkUnacknowledged;
        Ok(())
    })
    .map(|_| ())
}

fn record_verified_fork(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
    fork: &VerifiedNativeFork,
) -> Result<ConversationSyncOperation, String> {
    let (_, updated) = mutate_operation(repository, &operation.operation_id, |journaled| {
        let divergence = divergence_mut(journaled)?;
        let mut attempts = Vec::new();
        for executor in &divergence.available_executors {
            if *executor == fork.executor {
                attempts.push(CodexForkAttempt {
                    executor: *executor,
                    status: CodexForkAttemptStatus::Verified,
                });
                break;
            }
            attempts.push(CodexForkAttempt {
                executor: *executor,
                status: CodexForkAttemptStatus::DefinitiveNoFork,
            });
        }
        divergence.attempts = attempts;
        divergence.current_executor = None;
        divergence.successful_executor = Some(fork.executor);
        divergence.fork_provider_thread_id = Some(fork.provider_thread_id.clone());
        divergence.fork_transcript_path = Some(fork.transcript_path.to_string_lossy().into_owned());
        divergence.fork_transcript_sha256 = Some(fork.transcript_sha256.clone());
        divergence.fork_name_status = Some(map_name_status(&fork.name_status));
        journaled.phase = ConversationSyncPhase::ForkVerified;
        Ok(())
    })?;
    Ok(updated)
}

fn map_name_status(status: &CodexNameStatus) -> ConversationNameStatus {
    match status {
        CodexNameStatus::Applied => ConversationNameStatus::Applied,
        CodexNameStatus::UserOverride => ConversationNameStatus::UserOverride,
        CodexNameStatus::Unchanged => ConversationNameStatus::Unchanged,
        CodexNameStatus::Deferred => ConversationNameStatus::Deferred,
        CodexNameStatus::Failed(_) => ConversationNameStatus::Failed,
    }
}

fn verify_journaled_fork(
    operation: &ConversationSyncOperation,
    fork: &CodexForkExecutionRequest,
) -> Result<(), String> {
    let divergence = operation
        .divergence()
        .ok_or_else(|| "conversation operation is not a Codex divergence".to_string())?;
    let provider_thread_id = divergence
        .fork_provider_thread_id
        .as_deref()
        .ok_or_else(|| "verified Codex fork lacks a provider thread id".to_string())?;
    let transcript_path = divergence
        .fork_transcript_path
        .as_deref()
        .map(Path::new)
        .ok_or_else(|| "verified Codex fork lacks a transcript path".to_string())?;
    let transcript_sha256 = divergence
        .fork_transcript_sha256
        .as_deref()
        .ok_or_else(|| "verified Codex fork lacks a transcript digest".to_string())?;
    verify_recorded_fork(fork, provider_thread_id, transcript_path, transcript_sha256)
}

fn commit_preserved_fork_binding(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<ConversationSyncOperation, String> {
    let divergence = operation
        .divergence()
        .ok_or_else(|| "conversation operation is not a Codex divergence".to_string())?;
    let fork_provider_thread_id = divergence
        .fork_provider_thread_id
        .as_ref()
        .ok_or_else(|| "verified Codex fork lacks a provider thread id".to_string())?;
    let fork_transcript_path = divergence
        .fork_transcript_path
        .as_ref()
        .ok_or_else(|| "verified Codex fork lacks a transcript path".to_string())?;
    let fork_sha256 = divergence
        .fork_transcript_sha256
        .as_ref()
        .ok_or_else(|| "verified Codex fork lacks a transcript digest".to_string())?;
    let expected_stamp = ForkStamp {
        fork_thread_id: divergence.preserved_thread_id.clone(),
        fork_provider_thread_id: fork_provider_thread_id.clone(),
        fork_transcript_path: fork_transcript_path.clone(),
        lineage_digest: divergence.local_lineage_digest.clone(),
        semantic_digest: divergence.local_semantic_digest.clone(),
        message_count: divergence.local_message_count,
        forked_at: operation.updated_at,
    };
    let stamp_matches = |stamp: Option<&ForkStamp>| {
        stamp.is_some_and(|stamp| {
            stamp.fork_thread_id == expected_stamp.fork_thread_id
                && stamp.fork_provider_thread_id == expected_stamp.fork_provider_thread_id
                && stamp.fork_transcript_path == expected_stamp.fork_transcript_path
                && stamp.lineage_digest == expected_stamp.lineage_digest
                && stamp.semantic_digest == expected_stamp.semantic_digest
                && stamp.message_count == expected_stamp.message_count
        })
    };
    let current = repository.load_conversation_bindings()?;
    let source_scope = |binding: &LocalConversationBinding| {
        binding.host_id == operation.host_id
            && binding.replica_id == operation.replica_id
            && binding.local_project_id == operation.local_project_id
            && binding.provider_profile_id == operation.provider_profile_id
            && binding.thread_id == operation.thread_id
    };
    let existing = current.bindings.iter().find(|binding| {
        binding.host_id == operation.host_id
            && binding.replica_id == operation.replica_id
            && binding.local_project_id == operation.local_project_id
            && binding.provider_profile_id == operation.provider_profile_id
            && binding.thread_id == divergence.preserved_thread_id
    });
    let exact_existing = existing.is_some_and(|binding| {
        binding.forked_from.as_ref() == Some(&operation.thread_id)
            && binding.provider_thread_id.as_ref() == Some(fork_provider_thread_id)
            && binding.materialization_state == ConversationMaterializationState::Resumable
            && binding.last_observed_file_sha256.as_ref() == Some(fork_sha256)
            && binding.last_observed_lineage_digest.as_ref()
                == Some(&divergence.local_lineage_digest)
            && binding.last_observed_semantic_digest.as_ref()
                == Some(&divergence.local_semantic_digest)
            && binding.source_revision.is_none()
            && binding.source_manifest_sha256.is_none()
    }) && current
        .bindings
        .iter()
        .find(|binding| source_scope(binding))
        .is_some_and(|source| stamp_matches(source.fork_stamp.as_ref()));
    if existing.is_some() && !exact_existing {
        return Err("preserved conversation binding is occupied".to_string());
    }
    if !exact_existing {
        if current.revision != operation.expected_bindings_document_revision {
            return Err("conversation bindings changed after review".to_string());
        }
        repository.mutate_conversation_bindings(|document| {
            let source_index = document
                .bindings
                .iter()
                .position(|binding| source_scope(binding))
                .ok_or_else(|| "conversation divergence source binding disappeared".to_string())?;
            let source = &document.bindings[source_index];
            if source.revision != operation.expected_binding_revision.unwrap_or(u64::MAX)
                || source.provider_thread_id.as_deref()
                    != Some(&divergence.source_provider_thread_id)
            {
                return Err("conversation divergence source binding changed".to_string());
            }
            document.bindings.push(LocalConversationBinding {
                host_id: operation.host_id.clone(),
                replica_id: operation.replica_id.clone(),
                local_project_id: operation.local_project_id.clone(),
                provider_profile_id: operation.provider_profile_id.clone(),
                thread_id: divergence.preserved_thread_id.clone(),
                forked_from: Some(operation.thread_id.clone()),
                fork_stamp: None,
                provider_thread_id: Some(fork_provider_thread_id.clone()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(fork_sha256.clone()),
                last_observed_head_message_id: None,
                last_observed_lineage_digest: Some(divergence.local_lineage_digest.clone()),
                last_observed_semantic_digest: Some(divergence.local_semantic_digest.clone()),
                source_revision: None,
                source_manifest_sha256: None,
                revision: 0,
                created_at: operation.updated_at,
                updated_at: operation.updated_at,
            });
            let source = &mut document.bindings[source_index];
            source.fork_stamp = Some(expected_stamp.clone());
            source.revision = source.revision.saturating_add(1);
            source.updated_at = operation.updated_at;
            Ok(())
        })?;
    }
    let document = repository.load_conversation_bindings()?;
    let source_revision = document
        .bindings
        .iter()
        .find(|binding| source_scope(binding))
        .map(|binding| binding.revision);
    let (_, updated) = mutate_operation(repository, &operation.operation_id, |journaled| {
        journaled.expected_bindings_document_revision = document.revision;
        if let Some(revision) = source_revision {
            journaled.expected_binding_revision = Some(revision);
        }
        Ok(())
    })?;
    Ok(updated)
}

fn record_post_commit_presentation(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<ConversationSyncOperation, String> {
    let divergence = operation
        .divergence()
        .ok_or_else(|| "conversation operation is not a Codex divergence".to_string())?;
    let original_name_status = if divergence.desired_original_name.as_deref()
        == divergence.pinned_local_title.as_deref()
        || divergence.desired_original_name.is_none()
    {
        ConversationNameStatus::Unchanged
    } else {
        ConversationNameStatus::Deferred
    };
    let (_, updated) = mutate_operation(repository, &operation.operation_id, |journaled| {
        let divergence = divergence_mut(journaled)?;
        divergence.original_name_status = Some(original_name_status);
        divergence.inventory_refresh_status = CodexInventoryRefreshStatus::Deferred;
        journaled.phase = ConversationSyncPhase::OriginalNamingRecorded;
        Ok(())
    })?;
    Ok(updated)
}

fn post_commit_presentation_task(
    operation: &ConversationSyncOperation,
    fork: &CodexForkExecutionRequest,
) -> Result<Option<CodexPostCommitPresentationTask>, String> {
    let divergence = operation
        .divergence()
        .ok_or_else(|| "conversation operation is not a Codex divergence".to_string())?;
    let fork_rename = divergence
        .fork_provider_thread_id
        .as_ref()
        .filter(|_| divergence.fork_name_status == Some(ConversationNameStatus::Deferred))
        .map(|thread_id| CodexThreadRenameRequest {
            thread_id: thread_id.clone(),
            expected_current: divergence.pinned_local_title.clone(),
            desired: divergence.desired_fork_name.clone(),
        });
    let original_rename = divergence
        .desired_original_name
        .as_ref()
        .filter(|_| divergence.original_name_status == Some(ConversationNameStatus::Deferred))
        .map(|desired| CodexThreadRenameRequest {
            thread_id: divergence.source_provider_thread_id.clone(),
            expected_current: divergence.pinned_local_title.clone(),
            desired: desired.clone(),
        });
    if fork_rename.is_none()
        && original_rename.is_none()
        && divergence.inventory_refresh_status != CodexInventoryRefreshStatus::Deferred
    {
        return Ok(None);
    }
    Ok(Some(CodexPostCommitPresentationTask {
        operation_id: operation.operation_id.clone(),
        request: CodexPostCommitPresentationRequest {
            codex_executable: fork.codex_executable.clone(),
            codex_home: fork.codex_home.clone(),
            project_directory: fork.project_directory.clone(),
            fork_rename,
            original_rename,
        },
        records_receipt: operation.fork_origin() != Some(ForkOrigin::Manual),
    }))
}

fn complete_post_commit_presentation_task(
    repository: &LocalStateRepository,
    task: &CodexPostCommitPresentationTask,
) -> Result<(), String> {
    let outcome = complete_codex_post_commit_presentation(&task.request);
    if !task.records_receipt {
        return Ok(());
    }
    repository.mutate_materializations(|_, materializations| {
        let conversation = materializations
            .records
            .iter_mut()
            .flat_map(|record| record.receipts.iter_mut())
            .filter_map(|receipt| receipt.conversation.as_mut())
            .find(|conversation| conversation.journal_operation_id == task.operation_id)
            .ok_or_else(|| "deferred Codex presentation receipt disappeared".to_string())?;
        let divergence = conversation
            .divergence
            .as_mut()
            .ok_or_else(|| "deferred Codex presentation lacks a divergence receipt".to_string())?;
        if let Some(status) = outcome.fork_name_status.as_ref() {
            divergence.fork_name_status = Some(map_name_status(status));
        }
        divergence.original_name_status = Some(map_name_status(&outcome.original_name_status));
        divergence.inventory_refresh_status = if outcome.inventory_refreshed {
            CodexInventoryRefreshStatus::Refreshed
        } else {
            CodexInventoryRefreshStatus::Failed
        };
        Ok(())
    })
}

fn dispatch_post_commit_presentation(
    repository: LocalStateRepository,
    task: CodexPostCommitPresentationTask,
) {
    // Activity reloads immediately after a manual fork command returns. Finish
    // its best-effort rename and inventory refresh first so that reload cannot
    // race the inherited title. Pull presentation remains detached because its
    // durable receipt records the eventual outcome.
    if !task.records_receipt {
        let _ = complete_post_commit_presentation_task(&repository, &task);
        return;
    }
    #[cfg(not(test))]
    {
        let _ = std::thread::Builder::new()
            .name("mallard-codex-presentation".to_string())
            .spawn(move || {
                let _ = complete_post_commit_presentation_task(&repository, &task);
            });
    }
    #[cfg(test)]
    {
        let _ = (repository, task);
    }
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
        let is_codex_divergence = matches!(
            &operation.operation,
            ConversationSyncOperationKind::CodexDivergence(_)
        );
        let result = (|| -> Result<RecoveryOutcome, String> {
            if operation.direction == ConversationPlanDirection::Push {
                mark_recovery_required(repository, &operation_id)?;
                return Ok(RecoveryOutcome::RecoveryRequired);
            }
            if is_codex_divergence {
                if operation.phase == ConversationSyncPhase::Prepared {
                    if operation.targets.iter().all(staged_target_is_valid) {
                        operation = set_phase(
                            repository,
                            &operation_id,
                            ConversationSyncPhase::ReplacementStaged,
                            operation.updated_at,
                        )?;
                    } else if operation.targets.iter().all(target_is_uncommitted) {
                        rollback_prepared(repository, &operation)?;
                        return Ok(RecoveryOutcome::RolledBack);
                    } else {
                        return Ok(RecoveryOutcome::RecoveryRequired);
                    }
                }
                if matches!(
                    operation.phase,
                    ConversationSyncPhase::ForkUnacknowledged
                        | ConversationSyncPhase::RecoveryRequired
                ) {
                    return Ok(RecoveryOutcome::RecoveryRequired);
                }
                if operation.phase == ConversationSyncPhase::LocalCommitted {
                    let fork = codex_fork_request_for_operation(repository, &operation)?;
                    let presentation_task = post_commit_presentation_task(&operation, &fork)?;
                    cleanup_and_retire(repository, &operation_id)?;
                    if let Some(task) = presentation_task {
                        dispatch_post_commit_presentation(repository.clone(), task);
                    }
                    return Ok(RecoveryOutcome::Completed);
                }
                let fork = codex_fork_request_for_operation(repository, &operation)?;
                if operation.phase == ConversationSyncPhase::ReplacementStaged
                    && operation.divergence().is_some_and(|divergence| {
                        divergence.resolution == ConversationDivergenceResolution::UseCloudOnly
                    })
                    && verify_source_idle_and_title(&fork).is_err()
                {
                    rollback_prepared(repository, &operation)?;
                    return Ok(RecoveryOutcome::RolledBack);
                }
                let mut no_failpoints = NoConversationFailpoints;
                if operation.phase == ConversationSyncPhase::ForkAttemptPending {
                    let attempt = operation
                        .divergence()
                        .and_then(|divergence| divergence.attempts.last())
                        .ok_or_else(|| {
                            "pending Codex fork lacks its durable executor attempt".to_string()
                        })?;
                    return match attempt.status {
                        CodexForkAttemptStatus::Pending => {
                            record_fork_attempt(
                                repository,
                                &operation_id,
                                attempt.executor,
                                CodexForkAttemptStatus::Unacknowledged,
                            )?;
                            Ok(RecoveryOutcome::RecoveryRequired)
                        }
                        CodexForkAttemptStatus::DefinitiveNoFork => {
                            rollback_prepared(repository, &operation)?;
                            Ok(RecoveryOutcome::RolledBack)
                        }
                        _ => Err("pending Codex fork has an invalid durable executor outcome"
                            .to_string()),
                    };
                }
                if operation.phase == ConversationSyncPhase::ReplacementStaged {
                    // No executor was durably marked Pending, so no fork was
                    // invoked. Startup recovery is deliberately inert: roll
                    // back staging and let a user start a fresh operation.
                    rollback_prepared(repository, &operation)?;
                    return Ok(RecoveryOutcome::RolledBack);
                }
                if matches!(
                    operation.phase,
                    ConversationSyncPhase::ForkVerified
                        | ConversationSyncPhase::ForkNamingRecorded
                        | ConversationSyncPhase::LocalForkBound
                        | ConversationSyncPhase::ProviderCommitted
                        | ConversationSyncPhase::BindingsCommitted
                        | ConversationSyncPhase::OriginalNamingRecorded
                        | ConversationSyncPhase::LocalCommitted
                ) {
                    return match finish_verified_codex_divergence(
                        repository,
                        operation,
                        &fork,
                        &mut no_failpoints,
                    )? {
                        CodexDivergenceApplyOutcome::Committed(_)
                        | CodexDivergenceApplyOutcome::ManualForkCommitted { .. } => {
                            Ok(RecoveryOutcome::Completed)
                        }
                        CodexDivergenceApplyOutcome::ForkUnacknowledged { .. } => {
                            Ok(RecoveryOutcome::RecoveryRequired)
                        }
                    };
                }
            }
            match operation.phase {
                ConversationSyncPhase::Prepared => {
                    if operation.targets.iter().all(staged_target_is_valid) {
                        operation = set_phase(
                            repository,
                            &operation_id,
                            ConversationSyncPhase::ReplacementStaged,
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

            if operation.phase == ConversationSyncPhase::ReplacementStaged {
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
                let remains_pending = repository
                    .load_conversation_sync_journal()?
                    .operations
                    .iter()
                    .any(|candidate| candidate.operation_id == operation_id);
                if !remains_pending {
                    report.rolled_back.push(operation_id);
                    continue;
                }
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
    if !matches!(
        operation.operation,
        ConversationSyncOperationKind::ProviderMaterialization
            | ConversationSyncOperationKind::CodexDivergence(_)
    ) {
        return Err("provider materialization requires a materialization journal".to_string());
    }
    if operation.phase != ConversationSyncPhase::Prepared || !operation.targets.is_empty() {
        return Err("new conversation operation must begin Prepared without targets".to_string());
    }
    if request.targets.len() > 64 {
        return Err("conversation materialization has too many provider targets".to_string());
    }
    if operation.materialization_state == ConversationMaterializationState::Resumable
        && request.targets.is_empty()
        && operation.fork_origin() != Some(ForkOrigin::Manual)
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
        for (index, operation) in operations.iter().enumerate() {
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
            // One thread never carries two concurrent journal operations:
            // a pending checkpoint must be verified or canceled before any
            // new Push or Pull mutation of the same conversation begins.
            if journal
                .operations
                .iter()
                .chain(operations[..index].iter())
                .any(|existing| {
                    existing.local_project_id == operation.local_project_id
                        && existing.provider == operation.provider
                        && existing.thread_id == operation.thread_id
                })
            {
                return Err(format!(
                    "conversation thread '{}' already has a pending sync operation; \
                     resolve or retry it before syncing this conversation again",
                    operation.thread_id
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
    if operation.materialization_state == ConversationMaterializationState::StoredOnly {
        if !targets.is_empty() || operation.provider_thread_id.is_some() {
            return Err("stored-only conversation cannot publish provider files".to_string());
        }
        return Ok(());
    }
    if operation.fork_origin() == Some(ForkOrigin::Manual) {
        if !targets.is_empty() {
            return Err("manual fork operation writes no provider files".to_string());
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
    // The Codex index row is conditional presentation metadata, written
    // only when the cloud thread carries a real title, so creates and
    // replacements alike allow at most one fragment.
    if operation.provider == mallard_domain::Provider::Codex && indexes > 1 {
        return Err("Codex conversations allow at most one index fragment".to_string());
    }
    // Claude has no provider index: the projects directory listing is the
    // index, so a resumable Claude materialization is exactly one transcript.
    if operation.provider == mallard_domain::Provider::Claude && indexes != 0 {
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
            if operation.provider == mallard_domain::Provider::Claude =>
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
            let meta = crate::provider_capture::scan_jsonl_metadata(
                path,
                crate::provider_capture::Provider::Claude,
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
            if operation.provider != mallard_domain::Provider::Codex {
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
        if id == thread_id {
            // Rows Mallard appends must match the provider schema exactly;
            // pre-existing rows stay Codex's business. In particular, the
            // append-only index can contain multiple rename rows for another
            // thread ID.
            let named = object
                .get("thread_name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| {
                    !name.trim().is_empty()
                        && name.len() <= 1024
                        && !name.chars().any(char::is_control)
                });
            let updated = object
                .get("updated_at")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok());
            if !named || !updated {
                return Err(
                    "Codex index destination row must carry a thread name and an RFC 3339 \
                     update time"
                        .to_string(),
                );
            }
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

#[cfg(test)]
mod codex_index_fragment_tests {
    use super::validate_codex_index_fragment;

    #[test]
    fn accepts_repeated_pre_existing_ids_in_append_only_index() {
        let bytes = br#"{"id":"existing","thread_name":"Before"}
{"id":"existing","thread_name":"After"}
{"id":"destination","thread_name":"Imported","updated_at":"2026-07-31T22:00:03.000000Z"}
"#;

        validate_codex_index_fragment(bytes, "destination").unwrap();
    }

    #[test]
    fn rejects_more_than_one_destination_row() {
        let bytes = br#"{"id":"destination","thread_name":"Imported","updated_at":"2026-07-31T22:00:03.000000Z"}
{"id":"destination","thread_name":"Imported again","updated_at":"2026-07-31T22:01:03.000000Z"}
"#;

        assert!(validate_codex_index_fragment(bytes, "destination")
            .unwrap_err()
            .contains("exactly one destination"));
    }
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

fn record_publication_anchor(
    operation: &mut ConversationSyncOperation,
    publication: &ConversationCloudPublication,
) -> Result<(), String> {
    let anchor = operation
        .cloud
        .as_mut()
        .ok_or_else(|| "conversation operation lacks a cloud anchor".to_string())?;
    anchor.revision = publication.revision;
    anchor.commit_id = publication.commit_id.clone();
    anchor.manifest_sha256 = publication.manifest_sha256.clone();
    Ok(())
}

fn commit_binding(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<(), String> {
    let anchor = operation.cloud_anchor()?.clone();
    let current = repository.load_conversation_bindings()?;
    if binding_commit_is_complete(&current, operation) {
        return Ok(());
    }
    repository.mutate_conversation_bindings(|document| {
        if document.revision != operation.expected_bindings_document_revision {
            return Err("conversation bindings changed after review".to_string());
        }

        let binding_target_thread_id = operation.binding_target_thread_id();
        if let Some(binding) = document.bindings.iter_mut().find(|binding| {
            binding.host_id == operation.host_id
                && binding.replica_id == operation.replica_id
                && binding.local_project_id == operation.local_project_id
                && binding.provider_profile_id == operation.provider_profile_id
                && binding.thread_id == *binding_target_thread_id
        }) {
            if operation.expected_binding_revision.is_some()
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
            binding.source_revision = Some(anchor.revision);
            binding.source_manifest_sha256 = Some(anchor.manifest_sha256.clone());
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
                fork_stamp: None,
                provider_thread_id: operation.provider_thread_id.clone(),
                materialization_state: operation.materialization_state,
                last_observed_file_sha256: transcript_target_digest(operation),
                last_observed_head_message_id: None,
                last_observed_lineage_digest: operation.lineage_digest.clone(),
                last_observed_semantic_digest: operation.semantic_digest.clone(),
                source_revision: Some(anchor.revision),
                source_manifest_sha256: Some(anchor.manifest_sha256.clone()),
                revision: 0,
                created_at: operation.updated_at,
                updated_at: operation.updated_at,
            });
        }
        Ok(())
    })
}

fn binding_commit_is_complete(
    document: &mallard_domain::ConversationBindings,
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
        binding.provider_thread_id == operation.provider_thread_id
            && binding.forked_from == operation.binding_forked_from
            && binding.materialization_state == operation.materialization_state
            && binding.last_observed_file_sha256 == transcript_target_digest(operation)
            && binding.last_observed_lineage_digest == operation.lineage_digest
            && binding.last_observed_semantic_digest == operation.semantic_digest
            && binding.source_revision == operation.cloud.as_ref().map(|anchor| anchor.revision)
            && binding.source_manifest_sha256.as_deref()
                == operation
                    .cloud
                    .as_ref()
                    .map(|anchor| anchor.manifest_sha256.as_str())
    });
    materialized_exists
}

fn transcript_target_digest(operation: &ConversationSyncOperation) -> Option<String> {
    operation
        .targets
        .iter()
        .find(|target| target.role == ConversationFileRole::Transcript)
        .and_then(|target| target.staged_sha256.clone())
        .or_else(|| operation.expected_local_digest.clone())
}

fn commit_receipt(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
) -> Result<ConversationApplyReceipt, String> {
    let anchor = operation.cloud_anchor()?.clone();
    let receipt = conversation_receipt(operation, &anchor);
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
                || record.storage_id != anchor.storage_id
                || record.bundle_id != anchor.bundle_id
                || record.revision != anchor.revision
                || record.commit_id != anchor.commit_id
                || record.manifest_sha256 != anchor.manifest_sha256
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
                storage_id: anchor.storage_id.clone(),
                bundle_id: anchor.bundle_id.clone(),
                revision: anchor.revision,
                commit_id: anchor.commit_id.clone(),
                manifest_sha256: anchor.manifest_sha256.clone(),
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

fn conversation_receipt(
    operation: &ConversationSyncOperation,
    anchor: &ConversationCloudAnchor,
) -> ConversationApplyReceipt {
    ConversationApplyReceipt {
        host_id: operation.host_id.clone(),
        replica_id: operation.replica_id.clone(),
        thread_id: operation.thread_id.clone(),
        provider_profile_id: operation.provider_profile_id.clone(),
        provider_thread_id: operation.provider_thread_id.clone(),
        materialization_state: operation.materialization_state,
        lineage_digest: operation.lineage_digest.clone(),
        semantic_digest: operation.semantic_digest.clone(),
        local_file_sha256_after: transcript_target_digest(operation),
        cloud_revision: anchor.revision,
        cloud_commit_id: anchor.commit_id.clone(),
        cloud_manifest_sha256: anchor.manifest_sha256.clone(),
        published_origin_host_token: None,
        published_origin_replica_token: None,
        journal_operation_id: operation.operation_id.clone(),
        divergence: operation
            .divergence()
            .map(|divergence| CodexDivergenceReceipt {
                resolution: divergence.resolution,
                preserved_thread_id: divergence
                    .fork_provider_thread_id
                    .as_ref()
                    .map(|_| divergence.preserved_thread_id.clone()),
                preserved_provider_thread_id: divergence.fork_provider_thread_id.clone(),
                executor: divergence.successful_executor,
                desired_fork_name: divergence
                    .fork_provider_thread_id
                    .as_ref()
                    .map(|_| divergence.desired_fork_name.clone()),
                fork_name_status: divergence.fork_name_status,
                desired_original_name: divergence.desired_original_name.clone(),
                original_name_status: divergence.original_name_status,
                inventory_refresh_status: divergence.inventory_refresh_status,
            }),
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn manual_post_commit_presentation_finishes_before_dispatch_returns() {
        let temp = tempfile::tempdir().unwrap();
        let mallard_home = temp.path().join("mallard");
        let codex_home = temp.path().join("codex");
        let project = temp.path().join("project");
        fs::create_dir(&mallard_home).unwrap();
        fs::create_dir(&codex_home).unwrap();
        fs::create_dir(&project).unwrap();
        let repository = LocalStateRepository::from_home_dir(&mallard_home).unwrap();
        let executable = temp.path().join("fake-codex");
        let script = r#"#!/bin/sh
while IFS= read -r line
do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"id":1,"result":{}}'
      ;;
    *'"method":"thread/read"'*)
      printf '%s\n' '{"id":2,"result":{"thread":{"name":null}}}'
      ;;
    *'"method":"thread/name/set"'*)
      printf '%s\n' '{"id":3,"result":{}}'
      ;;
    *'"method":"thread/list"'*)
      printf '%s\n' complete > "$CODEX_HOME/manual-presentation-complete"
      printf '%s\n' '{"id":4,"result":{"data":[],"nextCursor":null}}'
      ;;
  esac
done
"#;
        fs::write(&executable, script).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let task = CodexPostCommitPresentationTask {
            operation_id: ConversationOperationId::parse("operation-manual-presentation").unwrap(),
            request: CodexPostCommitPresentationRequest {
                codex_executable: executable,
                codex_home: codex_home.clone(),
                project_directory: project,
                fork_rename: Some(CodexThreadRenameRequest {
                    thread_id: "fork-thread".to_string(),
                    expected_current: None,
                    desired: "Fork title".to_string(),
                }),
                original_rename: None,
            },
            records_receipt: false,
        };

        dispatch_post_commit_presentation(repository, task);

        assert_eq!(
            fs::read_to_string(codex_home.join("manual-presentation-complete")).unwrap(),
            "complete\n"
        );
    }
}
