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

use crate::conversation_codec::CodecLineage;
use crate::persistence::LocalStateRepository;
use mallard_domain::{
    conversation_resource_id, hex_lower, sha256_hex, ActionId, ActionStatus, ApplyReceipt,
    ConversationApplyReceipt, ConversationBindings, ConversationFileRole,
    ConversationJournalTarget, ConversationMaterializationState, ConversationOperationId,
    ConversationPlanDirection, ConversationSourceDisposition, ConversationSyncOperation,
    ConversationSyncOperationKind, MaterializationId, MaterializationRecord, MaterializationStatus,
    PullJournalPhase, PushJournalPhase, RestoreActionType, SyncConfig, TargetExpectation,
    LOCAL_SCHEMA,
};

const MAX_PROVIDER_TARGET_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationTargetPayload {
    pub target_path: PathBuf,
    pub role: ConversationFileRole,
    pub expected: TargetExpectation,
    /// The provider index row ids this payload rewrites; non-empty exactly
    /// for `ProviderIndexFragment` payloads.
    pub index_rows: Vec<String>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ConversationMaterializationRequest {
    pub operation: ConversationSyncOperation,
    pub targets: Vec<ConversationTargetPayload>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationFailpoint {
    BeforeJournal,
    AfterJournal,
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
    BeforeCloudPublication,
    AfterCloudHeadCas,
    BeforeHeadAdvanced,
    AfterHeadAdvanced,
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
    BeforeConfigCommit,
    AfterConfigCommit,
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
    pub revision: u64,
    pub commit_id: String,
    pub manifest_sha256: String,
}

fn push_phase(operation: &ConversationSyncOperation) -> Option<PushJournalPhase> {
    match &operation.operation {
        ConversationSyncOperationKind::CloudPublication(publication) => Some(publication.phase),
        ConversationSyncOperationKind::ProviderMaterialization(_) => None,
    }
}

fn pull_phase(operation: &ConversationSyncOperation) -> Option<PullJournalPhase> {
    match &operation.operation {
        ConversationSyncOperationKind::ProviderMaterialization(materialization) => {
            Some(materialization.phase)
        }
        ConversationSyncOperationKind::CloudPublication(_) => None,
    }
}

fn set_push_phase(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
    phase: PushJournalPhase,
    updated_at: u64,
) -> Result<ConversationSyncOperation, String> {
    let (_, updated) = mutate_operation(repository, operation_id, |operation| {
        match &mut operation.operation {
            ConversationSyncOperationKind::CloudPublication(publication) => {
                publication.phase = phase;
            }
            ConversationSyncOperationKind::ProviderMaterialization(_) => {
                return Err("push phase on a materialization operation".to_string());
            }
        }
        operation.updated_at = operation.updated_at.max(updated_at);
        Ok(())
    })?;
    Ok(updated)
}

fn set_pull_phase(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
    phase: PullJournalPhase,
    updated_at: u64,
) -> Result<ConversationSyncOperation, String> {
    let (_, updated) = mutate_operation(repository, operation_id, |operation| {
        match &mut operation.operation {
            ConversationSyncOperationKind::ProviderMaterialization(materialization) => {
                materialization.phase = phase;
            }
            ConversationSyncOperationKind::CloudPublication(_) => {
                return Err("pull phase on a publication operation".to_string());
            }
        }
        operation.updated_at = operation.updated_at.max(updated_at);
        Ok(())
    })?;
    Ok(updated)
}

/// Install one journaled whole-document after-state: idempotent when the
/// current document already equals the after-state, a plain replacement when
/// it still equals the before-state, and a typed corruption block otherwise.
fn install_journaled_bindings(
    repository: &LocalStateRepository,
    before: &ConversationBindings,
    after: &ConversationBindings,
) -> Result<(), String> {
    let current = repository.load_conversation_bindings()?;
    if &current == after {
        return Ok(());
    }
    if &current != before {
        return Err(
            "conversation bindings are neither the journaled before nor after state".to_string(),
        );
    }
    repository.install_conversation_bindings_snapshot(after.clone())
}

fn install_journaled_sync_config(
    repository: &LocalStateRepository,
    before: &SyncConfig,
    after: &SyncConfig,
) -> Result<(), String> {
    let current = repository.load_config()?;
    if &current == after {
        return Ok(());
    }
    if &current != before {
        return Err("sync config is neither the journaled before nor after state".to_string());
    }
    repository.install_journaled_config_snapshot(after.clone())
}

/// Complete journaled publication payloads with the exact coordinates the
/// fused publish installed. Only rows this batch wrote are rewritten: each
/// operation's after-document gets the batch checkpoints' accepted pins moved
/// from the reviewed-anchor placeholder to the published head, and each
/// successor's before-document is re-chained from its predecessor's patched
/// after-document so sequential installation matches disk exactly. The first
/// operation's before-document is the pristine pre-batch load and is never
/// touched; pre-existing rows outside the batch are never touched either,
/// even when their accepted pin happens to equal the reviewed anchor.
fn patch_publication_payload(
    payload: &mut mallard_domain::ConversationPublicationOperation,
    storage_id: &mallard_domain::StorageId,
    batch: &[(mallard_domain::SyncScopeKey, mallard_domain::ThreadId)],
    publication: &ConversationCloudPublication,
    chained_before: Option<&mallard_domain::SyncConfig>,
) {
    payload.candidate_commit_id = publication.commit_id.clone();
    payload.candidate_manifest_sha256 = publication.manifest_sha256.clone();
    if let Some(before) = chained_before {
        payload.sync_config_before = before.clone();
    }
    for link in payload
        .sync_config_after
        .links
        .iter_mut()
        .filter(|link| &link.storage_id == storage_id)
    {
        for (scope, thread_id) in batch {
            if let Some(accepted) = link
                .conversation_checkpoints
                .get_mut(scope)
                .and_then(|threads| threads.get_mut(thread_id))
                .and_then(|checkpoint| checkpoint.accepted.as_mut())
            {
                accepted.accepted_at_commit_id = publication.commit_id.clone();
                accepted.accepted_at_manifest_sha256 = publication.manifest_sha256.clone();
            }
        }
    }
}

/// Commit a publishing Push's local after-state and receipt from its
/// journaled documents, then retire the operation. `publication` carries the
/// exact head coordinates the CAS installed.
fn finish_cloud_publication(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
    publication: &ConversationCloudPublication,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<ConversationApplyReceipt, String> {
    let ConversationSyncOperationKind::CloudPublication(payload) = &operation.operation else {
        return Err("cloud publication finish requires a publication operation".to_string());
    };
    failpoints.hit(&ConversationFailpoint::BeforeBindingCommit)?;
    install_journaled_bindings(
        repository,
        &payload.bindings_before,
        &payload.bindings_after,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterBindingCommit)?;
    failpoints.hit(&ConversationFailpoint::BeforeConfigCommit)?;
    install_journaled_sync_config(
        repository,
        &payload.sync_config_before,
        &payload.sync_config_after,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterConfigCommit)?;
    let operation = set_push_phase(
        repository,
        &operation.operation_id,
        PushJournalPhase::LocalCommitted,
        operation.updated_at,
    )?;
    failpoints.hit(&ConversationFailpoint::BeforeReceipt)?;
    let receipt = commit_receipt(
        repository,
        &operation,
        publication.revision,
        &publication.commit_id,
        &publication.manifest_sha256,
    );
    let (receipt, terminal_phase) = match receipt {
        Ok(receipt) => (receipt, PushJournalPhase::Complete),
        Err(_) => {
            // The local commit is durable; regenerate the receipt from the
            // journal and terminalize with a warning instead of failing.
            let regenerated = conversation_receipt(
                &operation,
                publication.revision,
                &publication.commit_id,
                &publication.manifest_sha256,
            );
            (regenerated, PushJournalPhase::CompleteWithWarnings)
        }
    };
    failpoints.hit(&ConversationFailpoint::AfterReceipt)?;
    failpoints.hit(&ConversationFailpoint::BeforeLocalCommitted)?;
    set_push_phase(
        repository,
        &operation.operation_id,
        terminal_phase,
        operation.updated_at,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterLocalCommitted)?;
    failpoints.hit(&ConversationFailpoint::BeforeCleanup)?;
    cleanup_and_retire(repository, &operation.operation_id)?;
    failpoints.hit(&ConversationFailpoint::AfterCleanup)?;
    Ok(receipt)
}

/// Journal a reviewed Push, publish through the caller's production bundle
/// engine, and finalize the local receipt. `inspect_after_error` closes the
/// important CAS ambiguity: an object store may commit the head and then
/// return an I/O error.
#[cfg(any(test, feature = "test-support"))]
pub fn apply_conversation_cloud_publication(
    repository: &LocalStateRepository,
    operation: ConversationSyncOperation,
    publish: impl FnOnce() -> Result<ConversationCloudPublication, String>,
    inspect_after_error: impl FnOnce() -> Result<Option<ConversationCloudPublication>, String>,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<ConversationApplyReceipt, String> {
    let _host_guard = repository.acquire_host_operation()?;
    let receipts = apply_conversation_cloud_publications_locked(
        repository,
        vec![operation],
        publish,
        inspect_after_error,
        failpoints,
    )?;
    receipts
        .into_iter()
        .next()
        .ok_or_else(|| "cloud publication produced no receipt".to_string())
}

/// Publish a reviewed set of Push conversation actions while the command
/// orchestrator holds the host-operation guard across generic and
/// conversation publication. Candidate objects, the immutable manifest, and
/// the candidate commit are already uploaded (orphan-safe) by the caller;
/// this function journals the decision, advances the head once by CAS, and
/// installs the exact journaled local after-states.
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
        if push_phase(operation) != Some(PushJournalPhase::CandidateUploaded)
            || !operation.targets.is_empty()
        {
            return Err("cloud publication requires CandidateUploaded Push operations".to_string());
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

    failpoints.hit(&ConversationFailpoint::BeforeJournal)?;
    insert_operations(repository, &operations)?;
    failpoints.hit(&ConversationFailpoint::AfterJournal)?;
    failpoints.hit(&ConversationFailpoint::BeforeCloudPublication)?;
    let publication = match publish() {
        Ok(publication) => publication,
        Err(error) => match inspect_after_error()? {
            Some(publication) => publication,
            None => {
                for operation in &operations {
                    set_push_phase(
                        repository,
                        &operation.operation_id,
                        PushJournalPhase::AbortedCasLost,
                        operation.updated_at,
                    )?;
                    cleanup_and_retire(repository, &operation.operation_id)?;
                }
                return Err(error);
            }
        },
    };
    failpoints.hit(&ConversationFailpoint::AfterCloudHeadCas)?;
    failpoints.hit(&ConversationFailpoint::BeforeHeadAdvanced)?;
    let batch_pairs: Vec<(mallard_domain::SyncScopeKey, mallard_domain::ThreadId)> = operations
        .iter()
        .map(|operation| {
            (
                mallard_domain::SyncScopeKey::Provider(operation.provider.clone()),
                operation.thread_id.clone(),
            )
        })
        .collect();
    let journal = repository.load_conversation_sync_journal()?;
    repository.mutate_conversation_sync_journal(journal.revision, |journal| {
        let mut chained_before: Option<mallard_domain::SyncConfig> = None;
        for operation_id in &operation_ids {
            let operation = journal
                .operations
                .iter_mut()
                .find(|operation| &operation.operation_id == operation_id)
                .ok_or_else(|| format!("conversation operation '{}' disappeared", operation_id))?;
            match &mut operation.operation {
                ConversationSyncOperationKind::CloudPublication(publication_payload) => {
                    publication_payload.phase = PushJournalPhase::HeadAdvanced;
                    patch_publication_payload(
                        publication_payload,
                        &operation.cloud.storage_id,
                        &batch_pairs,
                        &publication,
                        chained_before.as_ref(),
                    );
                    chained_before = Some(publication_payload.sync_config_after.clone());
                }
                ConversationSyncOperationKind::ProviderMaterialization(_) => {
                    return Err("publication batch contains a materialization".to_string());
                }
            }
        }
        Ok(())
    })?;
    failpoints.hit(&ConversationFailpoint::AfterHeadAdvanced)?;

    let mut receipts = Vec::with_capacity(operation_ids.len());
    for operation_id in operation_ids {
        let operation = repository
            .load_conversation_sync_journal()?
            .operations
            .into_iter()
            .find(|operation| operation.operation_id == operation_id)
            .ok_or_else(|| format!("conversation operation '{}' disappeared", operation_id))?;
        receipts.push(finish_cloud_publication(
            repository,
            &operation,
            &publication,
            failpoints,
        )?);
    }
    Ok(receipts)
}

/// Recover Push operations with a caller-supplied immutable-head inspector.
/// The inspector must prove the candidate commit is the current head or a
/// verified descendant before returning a publication; orphan objects never
/// justify promotion.
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
    let mut no_failpoints = NoConversationFailpoints;
    for operation in operations {
        let Some(phase) = push_phase(&operation) else {
            continue;
        };
        let operation_id = operation.operation_id.clone();
        if phase.is_blocked() {
            report.recovery_required.push(operation_id);
            continue;
        }
        if phase.is_terminal() {
            cleanup_and_retire(repository, &operation_id)?;
            report.completed.push(operation_id);
            continue;
        }
        match phase {
            PushJournalPhase::CandidateUploaded => match inspect(&operation)? {
                Some(publication) => {
                    let operation = set_push_phase(
                        repository,
                        &operation_id,
                        PushJournalPhase::HeadAdvanced,
                        operation.updated_at,
                    )?;
                    finish_cloud_publication(
                        repository,
                        &operation,
                        &publication,
                        &mut no_failpoints,
                    )?;
                    report.completed.push(operation_id);
                }
                None => {
                    set_push_phase(
                        repository,
                        &operation_id,
                        PushJournalPhase::AbortedCasLost,
                        operation.updated_at,
                    )?;
                    cleanup_and_retire(repository, &operation_id)?;
                    report.rolled_back.push(operation_id);
                }
            },
            PushJournalPhase::HeadAdvanced | PushJournalPhase::LocalCommitted => {
                match inspect(&operation)? {
                    Some(publication) => {
                        finish_cloud_publication(
                            repository,
                            &operation,
                            &publication,
                            &mut no_failpoints,
                        )?;
                        report.completed.push(operation_id);
                    }
                    None => {
                        mark_recovery_blocked(repository, &operation_id)?;
                        report.recovery_required.push(operation_id);
                    }
                }
            }
            _ => unreachable!("terminal and blocked phases handled above"),
        }
    }
    Ok(report)
}

/// Apply one reviewed Pull materialization. All target expectations and local
/// scope revisions are checked before the TargetStaged record is written.
#[cfg(any(test, feature = "test-support"))]
pub fn apply_conversation_materialization(
    repository: &LocalStateRepository,
    request: ConversationMaterializationRequest,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<ConversationApplyReceipt, String> {
    let _host_guard = repository.acquire_host_operation()?;
    apply_conversation_materialization_locked(repository, request, failpoints)
}

/// Apply one reviewed materializing Pull (including keep-both) while the
/// command orchestrator holds the host-operation guard. The journaled
/// operation embeds the exact whole-document bindings and `SyncConfig`
/// before/after states; committing local state is installation, never
/// recomputation.
pub fn apply_conversation_materialization_locked(
    repository: &LocalStateRepository,
    request: ConversationMaterializationRequest,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<ConversationApplyReceipt, String> {
    request.operation.validate()?;
    if pull_phase(&request.operation) != Some(PullJournalPhase::TargetStaged) {
        return Err("conversation materialization must begin TargetStaged".to_string());
    }
    validate_operation_scope(repository, &request.operation)?;
    ensure_provider_target_parents(repository, &request.operation, &request.targets)?;
    let (operation, payloads) = prepare_operation(repository, request)?;
    validate_target_expectations(&operation.targets)?;

    failpoints.hit(&ConversationFailpoint::BeforeJournal)?;
    insert_operation(repository, &operation)?;
    failpoints.hit(&ConversationFailpoint::AfterJournal)?;

    for (index, bytes) in payloads.iter().enumerate() {
        write_intent_payload(repository, &operation.operation_id, index, bytes)?;
    }

    stage_targets(repository, &operation, failpoints)?;
    failpoints.hit(&ConversationFailpoint::BeforeStaged)?;
    failpoints.hit(&ConversationFailpoint::AfterStaged)?;
    finish_materialization(repository, operation, failpoints)
}

/// Drive a TargetStaged materialization to its terminal phase: commit
/// provider targets no-clobber, install the journaled document after-states,
/// and write the receipt.
fn finish_materialization(
    repository: &LocalStateRepository,
    operation: ConversationSyncOperation,
    failpoints: &mut impl ConversationFailpoints,
) -> Result<ConversationApplyReceipt, String> {
    let ConversationSyncOperationKind::ProviderMaterialization(payload) = &operation.operation
    else {
        return Err("materialization finish requires a materialization operation".to_string());
    };
    let payload = payload.clone();
    if let Err(error) = commit_provider_targets(repository, &operation.operation_id, failpoints) {
        // A foreign target collision aborts safely: no clobber, the original
        // graph preserved, committed Mallard targets recorded as recoverable
        // orphans through the terminal journal record. Every other failure
        // (crash, I/O) leaves the journal at its durable phase for recovery.
        if is_target_collision(&error) {
            set_pull_phase(
                repository,
                &operation.operation_id,
                PullJournalPhase::AbortedProviderCollision,
                operation.updated_at,
            )?;
            cleanup_and_retire(repository, &operation.operation_id)?;
        }
        return Err(error);
    }
    failpoints.hit(&ConversationFailpoint::BeforeProviderCommitted)?;
    let operation = set_pull_phase(
        repository,
        &operation.operation_id,
        PullJournalPhase::ProviderCommitted,
        operation.updated_at,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterProviderCommitted)?;

    failpoints.hit(&ConversationFailpoint::BeforeBindingCommit)?;
    install_journaled_bindings(
        repository,
        &payload.bindings_before,
        &payload.bindings_after,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterBindingCommit)?;
    failpoints.hit(&ConversationFailpoint::BeforeConfigCommit)?;
    install_journaled_sync_config(
        repository,
        &payload.sync_config_before,
        &payload.sync_config_after,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterConfigCommit)?;
    let operation = set_pull_phase(
        repository,
        &operation.operation_id,
        PullJournalPhase::LocalCommitted,
        operation.updated_at,
    )?;

    failpoints.hit(&ConversationFailpoint::BeforeReceipt)?;
    let anchor = operation.cloud.clone();
    let receipt = commit_receipt(
        repository,
        &operation,
        anchor.revision,
        &anchor.commit_id,
        &anchor.manifest_sha256,
    );
    let (receipt, terminal_phase) = match receipt {
        Ok(receipt) => (receipt, PullJournalPhase::Complete),
        Err(_) => {
            let regenerated = conversation_receipt(
                &operation,
                anchor.revision,
                &anchor.commit_id,
                &anchor.manifest_sha256,
            );
            (regenerated, PullJournalPhase::CompleteWithWarnings)
        }
    };
    failpoints.hit(&ConversationFailpoint::AfterReceipt)?;
    failpoints.hit(&ConversationFailpoint::BeforeLocalCommitted)?;
    set_pull_phase(
        repository,
        &operation.operation_id,
        terminal_phase,
        operation.updated_at,
    )?;
    failpoints.hit(&ConversationFailpoint::AfterLocalCommitted)?;
    failpoints.hit(&ConversationFailpoint::BeforeCleanup)?;
    cleanup_and_retire(repository, &operation.operation_id)?;
    failpoints.hit(&ConversationFailpoint::AfterCleanup)?;
    Ok(receipt)
}

/// Reconcile every durable nonterminal journal operation to one
/// deterministic fixed point. Pull operations roll forward from their last
/// durable phase or abort before any visible mutation; Push operations need
/// head inspection and are only marked for the caller's publication-recovery
/// entry point.
pub fn recover_conversation_materializations(
    repository: &LocalStateRepository,
) -> Result<ConversationRecoveryReport, String> {
    let _host_guard = repository.acquire_host_operation()?;
    let mut report = ConversationRecoveryReport::default();
    let operations = repository.load_conversation_sync_journal()?.operations;
    for operation in operations {
        let operation_id = operation.operation_id.clone();
        let result = (|| -> Result<RecoveryOutcome, String> {
            if operation.direction() == ConversationPlanDirection::Push {
                // Head inspection needs the caller's storage engine; startup
                // recovery only surfaces the pending operation.
                return Ok(RecoveryOutcome::RecoveryRequired);
            }
            let Some(phase) = pull_phase(&operation) else {
                return Ok(RecoveryOutcome::RecoveryRequired);
            };
            if phase.is_blocked() {
                return Ok(RecoveryOutcome::RecoveryRequired);
            }
            if phase.is_terminal() {
                cleanup_and_retire(repository, &operation_id)?;
                return Ok(RecoveryOutcome::Completed);
            }
            let mut no_failpoints = NoConversationFailpoints;
            match phase {
                PullJournalPhase::TargetStaged => {
                    // The journal is the durable commit decision: roll
                    // forward whenever the staged state can be completed —
                    // re-staging from intent payloads when a crash landed
                    // between the journal and the stage files.
                    let can_restage = operation.targets.iter().enumerate().all(|(index, _)| {
                        intent_payload_path(repository, &operation_id, index).is_file()
                    });
                    if operation.targets.iter().all(staged_target_is_valid)
                        || operation.targets.iter().any(|target| target.committed)
                        || can_restage
                    {
                        if !operation.targets.iter().all(staged_target_is_valid) {
                            stage_targets(repository, &operation, &mut no_failpoints)?;
                        }
                        finish_materialization(repository, operation, &mut no_failpoints)?;
                        Ok(RecoveryOutcome::Completed)
                    } else if operation.targets.iter().all(target_is_uncommitted) {
                        // Nothing visible was mutated and the payloads are
                        // gone: abort before commit.
                        set_pull_phase(
                            repository,
                            &operation_id,
                            PullJournalPhase::AbortedPrecommit,
                            operation.updated_at,
                        )?;
                        rollback_staged(repository, &operation)?;
                        Ok(RecoveryOutcome::RolledBack)
                    } else {
                        mark_recovery_blocked(repository, &operation_id)?;
                        Ok(RecoveryOutcome::RecoveryRequired)
                    }
                }
                PullJournalPhase::ProviderCommitted | PullJournalPhase::LocalCommitted => {
                    finish_materialization(repository, operation, &mut no_failpoints)?;
                    Ok(RecoveryOutcome::Completed)
                }
                _ => unreachable!("terminal and blocked phases handled above"),
            }
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
                let _ = mark_recovery_blocked(repository, &operation_id);
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
    if !matches!(
        operation.operation,
        ConversationSyncOperationKind::ProviderMaterialization(_)
    ) {
        return Err("provider materialization requires a materialization journal".to_string());
    }
    if !operation.targets.is_empty() {
        return Err("new conversation operation must begin without journaled targets".to_string());
    }
    if request.targets.len() > 64 {
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
        let staged_sha256 = sha256_hex(&payload.bytes);
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
            index_rows: payload.index_rows,
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
            // The claim is checked in both identity spaces — a retried
            // pull action mints fresh local identities, so only its cloud
            // id (and plan action) can correlate it with the journaled
            // first attempt. The cloud key is deliberately not
            // provider-qualified: cloud ids are bundle-global.
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
            if journal
                .operations
                .iter()
                .chain(operations[..index].iter())
                .any(|existing| {
                    existing.local_project_id == operation.local_project_id
                        && existing.cloud_thread_id == operation.cloud_thread_id
                })
            {
                return Err(format!(
                    "cloud thread '{}' already has a pending sync operation; \
                     resolve or retry it before syncing this conversation again",
                    operation.cloud_thread_id
                ));
            }
            if operation.plan_id.is_some()
                && operation.plan_action_id.is_some()
                && journal
                    .operations
                    .iter()
                    .chain(operations[..index].iter())
                    .any(|existing| {
                        existing.plan_id == operation.plan_id
                            && existing.plan_action_id == operation.plan_action_id
                    })
            {
                return Err(format!(
                    "conversation plan action '{}' already has a journaled operation",
                    operation
                        .plan_action_id
                        .as_ref()
                        .map(|action_id| action_id.as_str())
                        .unwrap_or("unknown")
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
    let adapter = crate::providers::ProviderRegistry::production().require(&operation.provider)?;
    if indexes > adapter.index_fragment_limit() {
        return Err(format!(
            "{} conversations allow at most {} index fragment(s)",
            operation.provider,
            adapter.index_fragment_limit()
        ));
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
        ConversationFileRole::Transcript => {
            // Typed verification through the registered adapter: the staged
            // bytes must re-analyze to the reviewed destination session in
            // the reviewed project with exactly the pinned lineage and
            // semantic digests. No provider ever falls back to generic
            // copying.
            let adapter =
                crate::providers::ProviderRegistry::production().require(&operation.provider)?;
            let analysis = adapter.analyze(&bytes);
            if analysis.source_provider_session_id.as_deref()
                != operation.provider_thread_id.as_deref()
            {
                return Err(format!(
                    "staged {} transcript has the wrong provider session id",
                    operation.provider
                ));
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
                return Err(format!(
                    "staged {} transcript has the wrong project cwd",
                    operation.provider
                ));
            }
            let ConversationSyncOperationKind::ProviderMaterialization(materialization) =
                &operation.operation
            else {
                return Err("staged transcript outside a materialization".to_string());
            };
            match analysis.lineage {
                CodecLineage::Complete {
                    lineage_digest,
                    semantic_digest,
                    ..
                } if lineage_digest == materialization.pinned_lineage_digest
                    && semantic_digest == materialization.pinned_semantic_digest => {}
                _ => {
                    return Err(format!(
                        "staged {} transcript changed lineage or semantic content",
                        operation.provider
                    ))
                }
            }
        }
        ConversationFileRole::ProviderIndexFragment => {
            // The journaled target names every row this fragment rewrites
            // (destination and, on keep-both, the preserved rename); the
            // domain guarantees the list is non-empty for this role.
            let adapter =
                crate::providers::ProviderRegistry::production().require(&operation.provider)?;
            for thread_id in &target.index_rows {
                adapter.verify_index_fragment(&bytes, thread_id)?;
            }
        }
        ConversationFileRole::Sidecar { .. } => {
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|_| "conversation sidecar is not valid JSON".to_string())?;
        }
    }
    Ok(())
}

/// The no-clobber verdicts from `commit_one_target` that prove a foreign
/// writer owns the target. Everything else is retryable.
fn is_target_collision(error: &str) -> bool {
    error.contains("cannot prove ownership")
        || error.contains("changed during apply")
        || error.contains("committed conversation provider target changed")
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

fn transcript_target_digest(operation: &ConversationSyncOperation) -> Option<String> {
    operation
        .targets
        .iter()
        .find(|target| target.role == ConversationFileRole::Transcript)
        .and_then(|target| target.staged_sha256.clone())
}

fn commit_receipt(
    repository: &LocalStateRepository,
    operation: &ConversationSyncOperation,
    cloud_revision: u64,
    cloud_commit_id: &str,
    cloud_manifest_sha256: &str,
) -> Result<ConversationApplyReceipt, String> {
    let anchor = operation.cloud.clone();
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
        let resource_id = conversation_resource_id(&operation.provider, &operation.thread_id)?;
        let action_type = match operation.direction() {
            ConversationPlanDirection::Push => RestoreActionType::PublishConversation,
            ConversationPlanDirection::Pull => RestoreActionType::MaterializeConversation,
        };
        let apply_receipt = ApplyReceipt {
            action_id,
            resource_id,
            action_type,
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
                revision: cloud_revision,
                commit_id: cloud_commit_id.to_string(),
                manifest_sha256: cloud_manifest_sha256.to_string(),
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
    cloud_revision: u64,
    cloud_commit_id: &str,
    cloud_manifest_sha256: &str,
) -> ConversationApplyReceipt {
    let (accepted_tail, preserved_thread_id, preserved_display_title, source_disposition) =
        match &operation.operation {
            ConversationSyncOperationKind::CloudPublication(publication) => (
                Some(publication.tail_after.clone()),
                None,
                None,
                Some(ConversationSourceDisposition::Retained),
            ),
            ConversationSyncOperationKind::ProviderMaterialization(materialization) => (
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
        journal_operation_id: operation.operation_id.clone(),
        accepted_tail,
        preserved_thread_id,
        preserved_display_title,
        source_disposition,
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

fn rollback_staged(
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

/// Gate the touched resources for explicit repair: the blocked phase is
/// durable and is neither retried nor released by routine recovery.
fn mark_recovery_blocked(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
) -> Result<(), String> {
    let (_, _) = mutate_operation(repository, operation_id, |operation| {
        match &mut operation.operation {
            ConversationSyncOperationKind::CloudPublication(publication) => {
                publication.phase = PushJournalPhase::RecoveryBlockedCorruption;
            }
            ConversationSyncOperationKind::ProviderMaterialization(materialization) => {
                materialization.phase = PullJournalPhase::RecoveryBlockedCorruption;
            }
        }
        Ok(())
    })?;
    Ok(())
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
