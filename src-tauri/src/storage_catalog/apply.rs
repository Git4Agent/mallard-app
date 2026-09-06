//! Approved catalog mutation: target-safe directory replacement, the
//! composite preservation transaction, the standalone Fork copy, the native
//! executor, and startup recovery.
//!
//! Every directory mutation is journaled before it changes the target, staged
//! on the target filesystem, digest-verified, backed up, activated by rename
//! with no-clobber claims, verified again, and recoverable from the same
//! journal. Recovery resumes the same operation and name; a retry never
//! mints a second copy. Every format-specific step — capture, materialize,
//! rename, verification — goes through the registered codec of the
//! directory's own coordinate; nothing here names a provider, kind, or
//! format.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mallard_domain::{hex_lower, update_length_prefixed, StorageId};
use sha2::{Digest, Sha256};

use super::adapters::{redact, PluginProcessRunner};
use super::codec::{read_directory_tree, CapturedFile, CapturedResource, TreeLimits};
use super::domain::{
    CatalogActionOutcome, CatalogActionResult, CatalogClaim, CatalogCoordinate, CatalogOperationId,
    CatalogOperationJournalEntry, CatalogOperationKind, CatalogOperationPhase, CatalogPlanActionId,
    CatalogPlanId, CatalogPreservationPin, CatalogPreservationWitness, CatalogPullAction,
    CatalogPullActionKind, CatalogPullDisposition, CatalogPullItem, CatalogResourceEnvelope,
    CatalogResourceForkPlan, CatalogResourceId, CatalogTargetConfig, CatalogTargetExpectation,
    NativePluginPlan, PluginApplyMode, MAX_CATALOG_FILES, MAX_CATALOG_FILE_BYTES,
    MAX_REVISION_PAYLOAD_BYTES,
};
use super::inventory::classify_directory;
use super::persistence::CatalogStateRepository;
use super::registry::{CatalogCapability, CatalogRegistry};

/// What a directory target currently holds: the registered version that
/// claims it and its exact content.
pub struct DirectoryObservation {
    pub capability: Arc<CatalogCapability>,
    pub envelope: CatalogResourceEnvelope,
    pub captured: CapturedResource,
}

/// Observe one directory target: `None` when absent, else the coordinate
/// that claims it and its current content. Classification starts from the
/// scope's own codec and falls through to the sibling versions sharing the
/// root, exactly as inventory does, so what a review saw is what apply sees.
pub fn observe_directory(
    registry: &CatalogRegistry,
    capability: &Arc<CatalogCapability>,
    root_class: &str,
    path: &Path,
) -> Result<Option<DirectoryObservation>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("inspect '{}': {}", path.display(), error)),
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            return Err(format!("'{}' is not a real directory", path.display()))
        }
        Ok(_) => {}
    }
    let (claimed, captured) = classify_directory(registry, capability, root_class, path)?;
    let envelope = captured.to_envelope(&claimed.coordinate());
    Ok(Some(DirectoryObservation {
        capability: claimed,
        envelope,
        captured,
    }))
}

fn observed_digest(
    registry: &CatalogRegistry,
    capability: &Arc<CatalogCapability>,
    root_class: &str,
    path: &Path,
) -> Result<Option<String>, String> {
    Ok(observe_directory(registry, capability, root_class, path)?
        .map(|observation| observation.envelope.content_sha256))
}

fn captured_tree_sha256(files: &BTreeMap<String, CapturedFile>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mallard-catalog-backup-tree-v1\0");
    for (path, file) in files {
        update_length_prefixed(&mut hasher, path.as_bytes());
        hasher.update(file.mode.to_be_bytes());
        update_length_prefixed(&mut hasher, &file.bytes);
    }
    hex_lower(&hasher.finalize())
}

fn backup_tree_sha256(path: &Path) -> Result<String, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err("catalog backup is not a real directory".to_string()),
        Err(error) => return Err(format!("inspect catalog backup: {error}")),
    }
    let files = read_directory_tree(
        path,
        &TreeLimits {
            max_files: MAX_CATALOG_FILES,
            max_file_bytes: MAX_CATALOG_FILE_BYTES,
            max_total_bytes: MAX_REVISION_PAYLOAD_BYTES,
        },
    )?;
    Ok(captured_tree_sha256(&files))
}

fn restore_verified_backup(
    backup: &Path,
    target: &Path,
    expected_tree_sha256: &str,
) -> Result<bool, String> {
    match fs::symlink_metadata(target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect restore target: {error}")),
        Ok(_) => return Ok(false),
    }
    if backup_tree_sha256(backup)? != expected_tree_sha256 {
        return Ok(false);
    }
    fs::rename(backup, target).map_err(|error| format!("restore catalog backup: {error}"))?;
    Ok(true)
}

/// One directory resource fully assembled in memory, ready to stage: the
/// codec of its own coordinate, its envelope, and its exact payload bytes.
pub struct IncomingDirectoryPayload {
    capability: Arc<CatalogCapability>,
    envelope: CatalogResourceEnvelope,
    files: BTreeMap<String, CapturedFile>,
}

impl IncomingDirectoryPayload {
    fn from_capture(capability: Arc<CatalogCapability>, captured: CapturedResource) -> Self {
        let envelope = captured.to_envelope(&capability.coordinate());
        Self {
            capability,
            envelope,
            files: captured.files,
        }
    }

    fn content_sha256(&self) -> &str {
        &self.envelope.content_sha256
    }

    fn materialize(&self, stage: &Path) -> Result<(), String> {
        self.capability
            .codec
            .materialize(stage, &self.envelope, &self.files)
    }

    /// The directory at `path` must recapture to exactly this resource.
    fn verify_at(
        &self,
        registry: &CatalogRegistry,
        root_class: &str,
        path: &Path,
    ) -> Result<(), String> {
        let observed = observe_directory(registry, &self.capability, root_class, path)?;
        let matches = observed.as_ref().is_some_and(|observation| {
            observation.envelope.coordinate() == self.envelope.coordinate()
                && observation.envelope.content_sha256 == self.envelope.content_sha256
        });
        if matches {
            Ok(())
        } else {
            Err(format!(
                "'{}' does not match its verified content",
                path.display()
            ))
        }
    }
}

/// The transformed local copy of an observed directory, through the codec
/// of the directory's own coordinate.
fn build_transformed_copy(
    source: &DirectoryObservation,
    copy_name: &str,
) -> Result<IncomingDirectoryPayload, String> {
    let renamed = source
        .capability
        .codec
        .rename(&source.captured, copy_name)?;
    Ok(IncomingDirectoryPayload::from_capture(
        source.capability.clone(),
        renamed,
    ))
}

fn expectation_matches(
    expected: &CatalogTargetExpectation,
    observed: Option<&str>,
) -> Result<(), String> {
    match (expected, observed) {
        (CatalogTargetExpectation::Absent, None) => Ok(()),
        (CatalogTargetExpectation::Sha256 { sha256 }, Some(observed)) if sha256 == observed => {
            Ok(())
        }
        _ => {
            Err("the target changed after review; refresh the catalog and review again".to_string())
        }
    }
}

fn journal_upsert(
    repository: &CatalogStateRepository,
    entry: CatalogOperationJournalEntry,
) -> Result<(), String> {
    repository.mutate_operations(|document| {
        document
            .operations
            .retain(|existing| existing.operation_id != entry.operation_id);
        document.operations.push(entry.clone());
        Ok(())
    })
}

fn journal_phase(
    repository: &CatalogStateRepository,
    operation_id: &CatalogOperationId,
    phase: CatalogOperationPhase,
    outcome: Option<CatalogActionOutcome>,
    message: Option<String>,
    now: u64,
) -> Result<(), String> {
    repository.mutate_operations(|document| {
        let entry = document
            .operations
            .iter_mut()
            .find(|entry| &entry.operation_id == operation_id)
            .ok_or("catalog operation journal entry disappeared")?;
        entry.phase = phase;
        entry.outcome = outcome;
        if message.is_some() {
            entry.message = message.clone();
        }
        entry.updated_at = now;
        Ok(())
    })
}

/// A journal entry with every optional field empty; callers fill what their
/// operation pins.
fn journal_entry(
    operation_id: CatalogOperationId,
    kind: CatalogOperationKind,
    coordinate: CatalogCoordinate,
    target: CatalogTargetConfig,
    now: u64,
) -> CatalogOperationJournalEntry {
    CatalogOperationJournalEntry {
        operation_id,
        kind,
        coordinate,
        storage_id: None,
        plan_id: None,
        action_id: None,
        target,
        target_path: None,
        backup_path: None,
        backup_tree_sha256: None,
        copy_target_path: None,
        copy_name: None,
        copy_resource_id: None,
        expected_target_sha256: None,
        incoming_content_sha256: None,
        copy_content_sha256: None,
        phase: CatalogOperationPhase::Journaled,
        outcome: None,
        message: None,
        created_at: now,
        updated_at: now,
    }
}

/// Atomically claim-and-activate a staged directory at a target that must not
/// exist: `create_dir` is the no-clobber claim, then the stage renames in.
fn activate_stage_no_clobber(stage: &Path, target: &Path) -> Result<(), String> {
    match fs::create_dir(target) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(format!(
                "'{}' appeared after review; nothing was replaced",
                target.display()
            ))
        }
        Err(error) => return Err(format!("claim '{}': {}", target.display(), error)),
    }
    #[cfg(unix)]
    {
        fs::rename(stage, target)
            .map_err(|error| format!("activate '{}': {}", target.display(), error))
    }
    #[cfg(not(unix))]
    {
        fs::remove_dir(target)
            .map_err(|error| format!("activate '{}': {}", target.display(), error))?;
        fs::rename(stage, target)
            .map_err(|error| format!("activate '{}': {}", target.display(), error))
    }
}

fn stage_dir_for(root: &Path, operation_id: &CatalogOperationId, suffix: &str) -> PathBuf {
    root.join(format!(".mallard-stage-{}{}", operation_id, suffix))
}

/// Stage payloads inside a correctly-named child of the hidden stage parent,
/// so name-bearing validation (pet ids, skill directories) sees the final
/// identity before activation.
fn staged_child(stage_parent: &Path, final_name: &str) -> Result<PathBuf, String> {
    let child = stage_parent.join(final_name);
    fs::create_dir_all(&child)
        .map_err(|error| format!("create stage '{}': {}", child.display(), error))?;
    Ok(child)
}

fn backup_target(
    repository: &CatalogStateRepository,
    plan_id: &CatalogPlanId,
    operation_id: &CatalogOperationId,
    target_path: &Path,
    expected_tree_sha256: &str,
    now: u64,
) -> Result<PathBuf, String> {
    if backup_tree_sha256(target_path)? != expected_tree_sha256 {
        return Err("the target changed before it could be backed up".to_string());
    }
    let name = target_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("backup target has no name")?;
    let preferred = repository
        .backup_dir(plan_id)?
        .join(format!("{}-{}", operation_id, name));
    record_backup_intent(
        repository,
        operation_id,
        &preferred,
        expected_tree_sha256,
        now,
    )?;
    match fs::rename(target_path, &preferred) {
        Ok(()) => {
            complete_backup(
                repository,
                operation_id,
                &preferred,
                expected_tree_sha256,
                now,
            )?;
            Ok(preferred)
        }
        Err(_) => {
            let sibling = target_path
                .parent()
                .ok_or("backup target has no parent")?
                .join(format!(".mallard-backup-{}-{}", operation_id, name));
            record_backup_intent(
                repository,
                operation_id,
                &sibling,
                expected_tree_sha256,
                now,
            )?;
            fs::rename(target_path, &sibling)
                .map_err(|error| format!("back up '{}': {}", target_path.display(), error))?;
            complete_backup(
                repository,
                operation_id,
                &sibling,
                expected_tree_sha256,
                now,
            )?;
            Ok(sibling)
        }
    }
}

fn record_backup_intent(
    repository: &CatalogStateRepository,
    operation_id: &CatalogOperationId,
    backup: &Path,
    backup_tree_sha256: &str,
    now: u64,
) -> Result<(), String> {
    repository.mutate_operations(|document| {
        let entry = document
            .operations
            .iter_mut()
            .find(|entry| &entry.operation_id == operation_id)
            .ok_or("catalog operation journal entry disappeared")?;
        entry.backup_path = Some(backup.to_string_lossy().to_string());
        entry.backup_tree_sha256 = Some(backup_tree_sha256.to_string());
        entry.updated_at = now;
        Ok(())
    })
}

fn complete_backup(
    repository: &CatalogStateRepository,
    operation_id: &CatalogOperationId,
    backup: &Path,
    expected_tree_sha256: &str,
    now: u64,
) -> Result<(), String> {
    if backup_tree_sha256(backup)? != expected_tree_sha256 {
        return Err("the catalog backup changed before it could be recorded".to_string());
    }
    journal_phase(
        repository,
        operation_id,
        CatalogOperationPhase::OriginalBackedUp,
        None,
        None,
        now,
    )
}

struct DirectoryReplaceRequest<'a> {
    storage_id: StorageId,
    plan_id: CatalogPlanId,
    action_id: CatalogPlanActionId,
    coordinate: CatalogCoordinate,
    target: &'a CatalogTargetConfig,
    target_path: &'a Path,
    expected: &'a CatalogTargetExpectation,
    incoming: &'a IncomingDirectoryPayload,
    now: u64,
}

/// One whole-directory replacement transaction. There is no per-file merge.
fn replace_directory_target(
    registry: &CatalogRegistry,
    repository: &CatalogStateRepository,
    request: DirectoryReplaceRequest<'_>,
) -> Result<(), String> {
    let root = request
        .target_path
        .parent()
        .ok_or("directory target has no parent")?;
    let root_class = request.target.decode()?.root_class;
    let _target_lock = repository.acquire_target_lock(root)?;

    let observed = observe_directory(
        registry,
        &request.incoming.capability,
        &root_class,
        request.target_path,
    )?;
    let observed_digest = observed
        .as_ref()
        .map(|observation| observation.envelope.content_sha256.as_str());
    expectation_matches(request.expected, observed_digest)?;
    let original_tree_sha256 = observed
        .as_ref()
        .map(|_| backup_tree_sha256(request.target_path))
        .transpose()?;

    let operation_id = CatalogOperationId::generate()?;
    let target_name = request
        .target_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("directory target has no name")?;
    let stage_parent = stage_dir_for(root, &operation_id, "");
    let staged = staged_child(&stage_parent, target_name).and_then(|stage| {
        request.incoming.materialize(&stage)?;
        request.incoming.verify_at(registry, &root_class, &stage)?;
        Ok(stage)
    });
    let stage = match staged {
        Ok(stage) => stage,
        Err(error) => {
            let _ = fs::remove_dir_all(&stage_parent);
            return Err(error);
        }
    };

    let mut entry = journal_entry(
        operation_id.clone(),
        CatalogOperationKind::DirectoryReplace,
        request.coordinate.clone(),
        request.target.clone(),
        request.now,
    );
    entry.storage_id = Some(request.storage_id.clone());
    entry.plan_id = Some(request.plan_id.clone());
    entry.action_id = Some(request.action_id.clone());
    entry.target_path = Some(request.target_path.to_string_lossy().to_string());
    entry.expected_target_sha256 = observed_digest.map(str::to_string);
    entry.incoming_content_sha256 = Some(request.incoming.content_sha256().to_string());
    journal_upsert(repository, entry)?;

    let backup = if let Some(original_tree_sha256) = original_tree_sha256.as_deref() {
        let backup = backup_target(
            repository,
            &request.plan_id,
            &operation_id,
            request.target_path,
            original_tree_sha256,
            request.now,
        )?;
        Some(backup)
    } else {
        None
    };

    let activation = activate_stage_no_clobber(&stage, request.target_path);
    let _ = fs::remove_dir_all(&stage_parent);
    if let Err(error) = activation {
        let restored = match (&backup, original_tree_sha256.as_deref()) {
            (Some(backup), Some(expected)) => {
                restore_verified_backup(backup, request.target_path, expected).unwrap_or(false)
            }
            _ => false,
        };
        let message = if restored {
            error.clone()
        } else {
            format!("{error}; the original backup was left unchanged")
        };
        journal_phase(
            repository,
            &operation_id,
            if restored {
                CatalogOperationPhase::RolledBack
            } else {
                CatalogOperationPhase::Failed
            },
            Some(if restored {
                CatalogActionOutcome::Failed
            } else {
                CatalogActionOutcome::Partial
            }),
            Some(message.clone()),
            request.now,
        )?;
        return Err(message);
    }

    if let Err(error) = request
        .incoming
        .verify_at(registry, &root_class, request.target_path)
    {
        journal_phase(
            repository,
            &operation_id,
            CatalogOperationPhase::Failed,
            Some(CatalogActionOutcome::Partial),
            Some(error.clone()),
            request.now,
        )?;
        return Err(error);
    }
    journal_phase(
        repository,
        &operation_id,
        CatalogOperationPhase::Completed,
        Some(CatalogActionOutcome::Installed),
        None,
        request.now,
    )?;
    Ok(())
}

struct PreservationRequest<'a> {
    storage_id: StorageId,
    plan_id: CatalogPlanId,
    action_id: CatalogPlanActionId,
    coordinate: CatalogCoordinate,
    resource_id: CatalogResourceId,
    target: &'a CatalogTargetConfig,
    target_path: &'a Path,
    pin: &'a CatalogPreservationPin,
    incoming: &'a IncomingDirectoryPayload,
    now: u64,
}

/// The composite preservation transaction: create the validated named copy,
/// witness it, then back up and overwrite the original.
fn preserve_and_overwrite(
    registry: &CatalogRegistry,
    repository: &CatalogStateRepository,
    request: PreservationRequest<'_>,
) -> Result<(), String> {
    let root = request
        .target_path
        .parent()
        .ok_or("preservation target has no parent")?;
    let root_class = request.target.decode()?.root_class;
    let _target_lock = repository.acquire_target_lock(root)?;

    // 1. Revalidate every pin against what actually sits at the target,
    // through whichever registered version claims it.
    let source = observe_directory(
        registry,
        &request.incoming.capability,
        &root_class,
        request.target_path,
    )?
    .ok_or("the diverged local version disappeared before preservation")?;
    if source.envelope.content_sha256 != request.pin.original_content_sha256 {
        return Err(
            "the local version changed after review; refresh the catalog and review again"
                .to_string(),
        );
    }
    let original_tree_sha256 = backup_tree_sha256(request.target_path)?;
    let copy_target = PathBuf::from(&request.pin.copy_target_path);
    let operation_id = CatalogOperationId::generate()?;

    // Reuse a valid witness from a prior partial run instead of copying
    // again; a missing or modified copy invalidates it below.
    let existing_witness = repository
        .load_preservations()?
        .witnesses
        .into_iter()
        .find(|witness| {
            witness.source_target == *request.target
                && witness.local_source_sha256 == request.pin.original_content_sha256
                && witness.storage_source_sha256 == request.pin.storage_content_sha256
                && witness.preserved_content_sha256 == request.pin.transformed_content_sha256
        });
    let copy_exists = fs::symlink_metadata(&copy_target).is_ok();
    let witness_valid = existing_witness.is_some()
        && copy_exists
        && observed_digest(
            registry,
            &request.incoming.capability,
            &root_class,
            &copy_target,
        )
        .ok()
        .flatten()
        .as_deref()
            == Some(request.pin.transformed_content_sha256.as_str());
    if copy_exists && !witness_valid {
        return Err(format!(
            "'{}' appeared after review; nothing was replaced",
            copy_target.display()
        ));
    }

    // 2-3. Stage the transformed copy and the storage version, each under a
    // hidden stage parent but inside a child carrying its final name.
    let target_name = request
        .target_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("preservation target has no name")?;
    let copy_stage_parent = stage_dir_for(root, &operation_id, "-copy");
    let incoming_stage_parent = stage_dir_for(root, &operation_id, "-incoming");
    let cleanup = || {
        let _ = fs::remove_dir_all(&copy_stage_parent);
        let _ = fs::remove_dir_all(&incoming_stage_parent);
    };
    let staged = (|| -> Result<(PathBuf, PathBuf), String> {
        let copy_stage = if witness_valid {
            copy_stage_parent.join(&request.pin.copy_name)
        } else {
            let copy_stage = staged_child(&copy_stage_parent, &request.pin.copy_name)?;
            let transformed = build_transformed_copy(&source, &request.pin.copy_name)?;
            if transformed.content_sha256() != request.pin.transformed_content_sha256 {
                return Err("the transformed copy failed digest validation".to_string());
            }
            transformed.materialize(&copy_stage)?;
            transformed.verify_at(registry, &root_class, &copy_stage)?;
            copy_stage
        };
        let incoming_stage = staged_child(&incoming_stage_parent, target_name)?;
        request.incoming.materialize(&incoming_stage)?;
        request
            .incoming
            .verify_at(registry, &root_class, &incoming_stage)?;
        Ok((copy_stage, incoming_stage))
    })();
    let (copy_stage, incoming_stage) = match staged {
        Ok(stages) => stages,
        Err(error) => {
            cleanup();
            return Err(error);
        }
    };

    // 5. Journal the pinned operation and chosen name.
    let mut entry = journal_entry(
        operation_id.clone(),
        CatalogOperationKind::Preservation,
        request.coordinate.clone(),
        request.target.clone(),
        request.now,
    );
    entry.storage_id = Some(request.storage_id.clone());
    entry.plan_id = Some(request.plan_id.clone());
    entry.action_id = Some(request.action_id.clone());
    entry.target_path = Some(request.target_path.to_string_lossy().to_string());
    entry.copy_target_path = Some(request.pin.copy_target_path.clone());
    entry.copy_name = Some(request.pin.copy_name.clone());
    entry.copy_resource_id = Some(request.pin.copy_resource_id.clone());
    entry.expected_target_sha256 = Some(request.pin.original_content_sha256.clone());
    entry.incoming_content_sha256 = Some(request.pin.storage_content_sha256.clone());
    entry.copy_content_sha256 = Some(request.pin.transformed_content_sha256.clone());
    journal_upsert(repository, entry)?;

    // 6. Activate the copy with no-clobber semantics and verify it.
    if !witness_valid {
        if let Err(error) = activate_stage_no_clobber(&copy_stage, &copy_target) {
            cleanup();
            journal_phase(
                repository,
                &operation_id,
                CatalogOperationPhase::Failed,
                Some(CatalogActionOutcome::Failed),
                Some(error.clone()),
                request.now,
            )?;
            return Err(error);
        }
        let _ = fs::remove_dir_all(&copy_stage_parent);
        let activated = observed_digest(
            registry,
            &request.incoming.capability,
            &root_class,
            &copy_target,
        )?;
        if activated.as_deref() != Some(request.pin.transformed_content_sha256.as_str()) {
            journal_phase(
                repository,
                &operation_id,
                CatalogOperationPhase::Failed,
                Some(CatalogActionOutcome::Partial),
                Some("the preserved copy failed verification".to_string()),
                request.now,
            )?;
            return Err("the preserved copy failed verification".to_string());
        }
    }
    journal_phase(
        repository,
        &operation_id,
        CatalogOperationPhase::CopyActivated,
        None,
        None,
        request.now,
    )?;

    // 7. Persist the preservation witness before changing the original.
    let preserved_target = request.target.with_member(&request.pin.copy_name)?;
    repository.mutate_preservations(|document| {
        document.witnesses.retain(|witness| {
            !(witness.source_target == *request.target
                && witness.storage_source_sha256 == request.pin.storage_content_sha256)
        });
        document.witnesses.push(CatalogPreservationWitness {
            storage_id: request.storage_id.clone(),
            coordinate: request.coordinate.clone(),
            source_resource_id: request.resource_id.clone(),
            source_target: request.target.clone(),
            local_source_sha256: request.pin.original_content_sha256.clone(),
            storage_source_sha256: request.pin.storage_content_sha256.clone(),
            preserved_resource_id: request.pin.copy_resource_id.clone(),
            preserved_target: preserved_target.clone(),
            preserved_content_sha256: request.pin.transformed_content_sha256.clone(),
            operation_id: operation_id.clone(),
            created_at: request.now,
        });
        Ok(())
    })?;
    journal_phase(
        repository,
        &operation_id,
        CatalogOperationPhase::WitnessRecorded,
        None,
        None,
        request.now,
    )?;

    // 8. Back up the original and activate the storage version.
    let backup = backup_target(
        repository,
        &request.plan_id,
        &operation_id,
        request.target_path,
        &original_tree_sha256,
        request.now,
    )?;
    let activation = activate_stage_no_clobber(&incoming_stage, request.target_path);
    let _ = fs::remove_dir_all(&incoming_stage_parent);
    if let Err(error) = activation {
        let restored = restore_verified_backup(&backup, request.target_path, &original_tree_sha256)
            .unwrap_or(false);
        let message = if restored {
            error.clone()
        } else {
            format!("{error}; the original backup was left unchanged")
        };
        journal_phase(
            repository,
            &operation_id,
            if restored {
                CatalogOperationPhase::RolledBack
            } else {
                CatalogOperationPhase::Failed
            },
            Some(if restored {
                CatalogActionOutcome::Failed
            } else {
                CatalogActionOutcome::Partial
            }),
            Some(message.clone()),
            request.now,
        )?;
        return Err(message);
    }
    if let Err(error) = request
        .incoming
        .verify_at(registry, &root_class, request.target_path)
    {
        journal_phase(
            repository,
            &operation_id,
            CatalogOperationPhase::Failed,
            Some(CatalogActionOutcome::Partial),
            Some(error.clone()),
            request.now,
        )?;
        return Err(error);
    }

    // 9. Complete the journal.
    journal_phase(
        repository,
        &operation_id,
        CatalogOperationPhase::Completed,
        Some(CatalogActionOutcome::PreservedAndUpdated),
        None,
        request.now,
    )?;
    Ok(())
}

/// Apply the standalone resource Fork plan: the same semantic rename,
/// staging, no-clobber activation, verification, journal, and recovery
/// primitive as Pull preservation, ending after the verified copy. It never
/// overwrites another target and never records a Pull preservation witness.
pub fn apply_resource_fork(
    registry: &CatalogRegistry,
    repository: &CatalogStateRepository,
    capability: &Arc<CatalogCapability>,
    plan: &CatalogResourceForkPlan,
    now: u64,
) -> Result<CatalogActionResult, String> {
    plan.validate()?;
    if now < plan.created_at || now > plan.expires_at {
        return Err("the fork plan expired; review it again".to_string());
    }
    let source_path = PathBuf::from(&plan.source_path);
    let copy_target = PathBuf::from(&plan.copy_target_path);
    let root = copy_target
        .parent()
        .ok_or("fork copy target has no parent")?;
    let root_class = plan.source_target.decode()?.root_class;
    let _target_lock = repository.acquire_target_lock(root)?;

    let source = observe_directory(registry, capability, &root_class, &source_path)?
        .ok_or("the fork source disappeared; refresh and review again")?;
    if source.envelope.content_sha256 != plan.source_content_sha256 {
        return Err("the fork source changed after review; review it again".to_string());
    }

    let operation_id = CatalogOperationId::generate()?;
    let stage_parent = stage_dir_for(root, &operation_id, "-fork");
    let staged = staged_child(&stage_parent, &plan.copy_name).and_then(|stage| {
        let payload = build_transformed_copy(&source, &plan.copy_name)?;
        if payload.content_sha256() != plan.transformed_content_sha256 {
            return Err("the fork copy failed digest validation".to_string());
        }
        payload.materialize(&stage)?;
        payload.verify_at(registry, &root_class, &stage)?;
        Ok(stage)
    });
    let stage = match staged {
        Ok(stage) => stage,
        Err(error) => {
            let _ = fs::remove_dir_all(&stage_parent);
            return Err(error);
        }
    };

    let mut entry = journal_entry(
        operation_id.clone(),
        CatalogOperationKind::Fork,
        plan.source_coordinate.clone(),
        plan.source_target.clone(),
        now,
    );
    entry.plan_id = Some(plan.plan_id.clone());
    entry.target_path = Some(plan.source_path.clone());
    entry.copy_target_path = Some(plan.copy_target_path.clone());
    entry.copy_name = Some(plan.copy_name.clone());
    entry.copy_resource_id = Some(plan.copy_resource_id.clone());
    entry.expected_target_sha256 = Some(plan.source_content_sha256.clone());
    entry.copy_content_sha256 = Some(plan.transformed_content_sha256.clone());
    journal_upsert(repository, entry)?;

    let activation = activate_stage_no_clobber(&stage, &copy_target);
    let _ = fs::remove_dir_all(&stage_parent);
    if let Err(error) = activation {
        journal_phase(
            repository,
            &operation_id,
            CatalogOperationPhase::Failed,
            Some(CatalogActionOutcome::Failed),
            Some(error.clone()),
            now,
        )?;
        return Err(error);
    }
    let activated = observed_digest(registry, capability, &root_class, &copy_target)?;
    if activated.as_deref() != Some(plan.transformed_content_sha256.as_str()) {
        journal_phase(
            repository,
            &operation_id,
            CatalogOperationPhase::Failed,
            Some(CatalogActionOutcome::Partial),
            Some("the fork copy failed verification".to_string()),
            now,
        )?;
        return Err("the fork copy failed verification".to_string());
    }
    journal_phase(
        repository,
        &operation_id,
        CatalogOperationPhase::Completed,
        Some(CatalogActionOutcome::Forked),
        Some(format!(
            "Created '{}' beside '{}'",
            plan.copy_name, plan.source_display_name
        )),
        now,
    )?;
    Ok(CatalogActionResult {
        resource_id: plan.copy_resource_id.clone(),
        action_id: None,
        display_name: plan.copy_name.clone(),
        outcome: CatalogActionOutcome::Forked,
        message: Some(format!(
            "Forked from '{}'; local-only until you push it",
            plan.source_display_name
        )),
    })
}

/// Execute one approved native operation and verify the result through the
/// adapter. A clean exit with failed verification is `partial`, never
/// success.
#[allow(clippy::too_many_arguments)]
fn execute_native_action(
    repository: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    capability: &CatalogCapability,
    storage_id: &StorageId,
    plan_id: &CatalogPlanId,
    action: &CatalogPullAction,
    native: &NativePluginPlan,
    resource: &CatalogResourceEnvelope,
    home: &Path,
    now: u64,
) -> Result<CatalogActionOutcome, String> {
    let adapter = capability.adapter.as_ref();
    let _target_lock = repository.acquire_target_lock(home)?;
    let current = adapter
        .discover_native(home, runner)
        .map_err(|blocker| blocker.to_string())?;
    let current_digest = adapter.native_state(resource, &current)?;
    expectation_matches(&action.expected_target_state, current_digest.as_deref())?;

    let operation_id = CatalogOperationId::generate()?;
    let mut entry = journal_entry(
        operation_id.clone(),
        CatalogOperationKind::NativeApply,
        capability.coordinate(),
        action.target.clone(),
        now,
    );
    entry.storage_id = Some(storage_id.clone());
    entry.plan_id = Some(plan_id.clone());
    entry.action_id = Some(action.action_id.clone());
    entry.incoming_content_sha256 = Some(action.incoming_content_sha256.clone());
    journal_upsert(repository, entry)?;
    let fail = |error: String| {
        journal_phase(
            repository,
            &operation_id,
            CatalogOperationPhase::Failed,
            Some(CatalogActionOutcome::Failed),
            Some(error.clone()),
            now,
        )
        .and(Err(error))
    };

    let mut argv_sets: Vec<&Vec<String>> = native.marketplace_argv.iter().collect();
    argv_sets.push(&native.argv);
    for argv in argv_sets {
        let output = match runner.run(&native.provider, home, argv) {
            Ok(output) => output,
            Err(error) => return fail(error),
        };
        if !output.success {
            let detail = redact(output.stderr.trim());
            let message = format!(
                "{} exited unsuccessfully: {}",
                native.provider,
                if detail.is_empty() {
                    redact(output.stdout.trim())
                } else {
                    detail
                }
            );
            return fail(message);
        }
    }

    // Post-exit re-inventory: the resource must be present AND the resulting
    // portable intent must match what was approved. A clean exit that leaves
    // a different intent installed is partial, never success. Presence and
    // intent are read from one snapshot.
    let after = adapter
        .discover_native(home, runner)
        .map_err(|blocker| blocker.to_string())?;
    let display_name = capability
        .codec
        .display_name(resource)
        .unwrap_or_else(|_| resource.resource_id.to_string());
    match adapter.native_state(resource, &after)? {
        Some(digest) if digest == action.incoming_content_sha256 => {
            let outcome = match native.mode {
                PluginApplyMode::InstallMissing => CatalogActionOutcome::Installed,
                PluginApplyMode::OverwriteExisting => CatalogActionOutcome::Updated,
            };
            journal_phase(
                repository,
                &operation_id,
                CatalogOperationPhase::Completed,
                Some(outcome),
                None,
                now,
            )?;
            Ok(outcome)
        }
        installed => {
            let detail = if installed.is_some() {
                format!(
                    "{} reports '{}' installed, but its portable intent does not match the approved catalog intent yet",
                    native.provider, display_name
                )
            } else {
                format!(
                    "{} does not report '{}' installed after the operation",
                    native.provider, display_name
                )
            };
            journal_phase(
                repository,
                &operation_id,
                CatalogOperationPhase::Completed,
                Some(CatalogActionOutcome::Partial),
                Some(detail),
                now,
            )?;
            Ok(CatalogActionOutcome::Partial)
        }
    }
}

/// Fetch one verified payload object by digest and exact size.
pub type PayloadFetcher<'a> = dyn Fn(&str, u64) -> Result<Vec<u8>, String> + 'a;

pub struct PullApplyEnvironment<'a> {
    pub repository: &'a CatalogStateRepository,
    pub runner: &'a dyn PluginProcessRunner,
    pub registry: &'a CatalogRegistry,
    pub capability: Arc<CatalogCapability>,
    pub fetch_payload: &'a PayloadFetcher<'a>,
}

/// The storage version of one directory resource, fully fetched and verified
/// against its envelope before any local mutation.
fn incoming_payload_for(
    capability: &Arc<CatalogCapability>,
    resource: &CatalogResourceEnvelope,
    fetch: &PayloadFetcher<'_>,
) -> Result<IncomingDirectoryPayload, String> {
    capability.codec.validate(resource)?;
    let mut files = BTreeMap::new();
    for file in &resource.files {
        files.insert(
            file.path.clone(),
            CapturedFile {
                bytes: fetch(&file.sha256, file.size)?,
                mode: file.mode,
            },
        );
    }
    // The assembled bytes must reproduce the stored envelope exactly.
    let rebuilt = CapturedResource {
        identity: resource.identity.clone(),
        descriptor: resource.descriptor.clone(),
        files: files.clone(),
        display_name: String::new(),
        warnings: Vec::new(),
        notes: Vec::new(),
    }
    .to_envelope(&resource.coordinate());
    if rebuilt.content_sha256 != resource.content_sha256 {
        return Err("the fetched payload does not match the stored resource".to_string());
    }
    Ok(IncomingDirectoryPayload {
        capability: capability.clone(),
        envelope: resource.clone(),
        files,
    })
}

/// Execute the single approved action (or explicit Skip) for one Pull item.
/// The command layer advances the accepted base for a resource only after
/// this returns a non-failed, non-partial outcome; explicit Skip is a
/// successful accept-without-mutation decision.
pub fn execute_scope_pull_item(
    environment: &PullApplyEnvironment<'_>,
    storage_id: &StorageId,
    plan_id: &CatalogPlanId,
    manifest_resources: &BTreeMap<CatalogResourceId, CatalogResourceEnvelope>,
    item: &CatalogPullItem,
    approved: Option<&CatalogPullAction>,
    now: u64,
) -> CatalogActionResult {
    let display_name = item.display_name.clone();
    let fixed = match &item.disposition {
        CatalogPullDisposition::AlreadyMatching => {
            Some((CatalogActionOutcome::AlreadyMatching, item.detail.clone()))
        }
        CatalogPullDisposition::KeptLocal => {
            Some((CatalogActionOutcome::KeptLocal, item.detail.clone()))
        }
        CatalogPullDisposition::Manual { message } => {
            Some((CatalogActionOutcome::Manual, Some(message.clone())))
        }
        CatalogPullDisposition::Blocked { reason } => {
            Some((CatalogActionOutcome::Failed, Some(reason.clone())))
        }
        CatalogPullDisposition::Executable => None,
    };
    if let Some((outcome, message)) = fixed {
        return CatalogActionResult {
            resource_id: item.resource_id.clone(),
            action_id: None,
            display_name,
            outcome,
            message,
        };
    }
    let Some(action) = approved else {
        return CatalogActionResult {
            resource_id: item.resource_id.clone(),
            action_id: None,
            display_name,
            outcome: CatalogActionOutcome::Skipped,
            message: Some("Skipped; local content was left unchanged".to_string()),
        };
    };
    match execute_one_action(
        environment,
        storage_id,
        plan_id,
        manifest_resources,
        action,
        now,
    ) {
        Ok(outcome) => CatalogActionResult {
            resource_id: item.resource_id.clone(),
            action_id: Some(action.action_id.clone()),
            display_name,
            outcome,
            message: action
                .kind
                .preservation()
                .map(|pin| format!("Kept your version as '{}'", pin.copy_name)),
        },
        Err(message) => CatalogActionResult {
            resource_id: item.resource_id.clone(),
            action_id: Some(action.action_id.clone()),
            display_name,
            outcome: CatalogActionOutcome::Failed,
            message: Some(message),
        },
    }
}

fn execute_one_action(
    environment: &PullApplyEnvironment<'_>,
    storage_id: &StorageId,
    plan_id: &CatalogPlanId,
    manifest_resources: &BTreeMap<CatalogResourceId, CatalogResourceEnvelope>,
    action: &CatalogPullAction,
    now: u64,
) -> Result<CatalogActionOutcome, String> {
    let resource = manifest_resources
        .get(&action.resource_id)
        .ok_or("the resource left the catalog after review")?;
    let coordinate = environment.capability.coordinate();
    if resource.coordinate() != coordinate {
        return Err("the resource changed coordinate after review".to_string());
    }
    let claim = |outcome: CatalogActionOutcome| {
        record_claim(
            environment.repository,
            storage_id,
            &coordinate,
            &action.target,
            &action.resource_id,
            &action.incoming_content_sha256,
            outcome,
            plan_id,
            now,
        )
    };

    if let Some(native) = &action.native {
        let home = PathBuf::from(action.target.decode()?.canonical_root);
        let outcome = execute_native_action(
            environment.repository,
            environment.runner,
            &environment.capability,
            storage_id,
            plan_id,
            action,
            native,
            resource,
            &home,
            now,
        )?;
        if !matches!(
            outcome,
            CatalogActionOutcome::Failed | CatalogActionOutcome::Partial
        ) {
            claim(outcome)?;
        }
        return Ok(outcome);
    }

    let incoming =
        incoming_payload_for(&environment.capability, resource, environment.fetch_payload)?;
    let target_path = action
        .target_path
        .as_ref()
        .ok_or("directory action lost its target path")?;
    match &action.kind {
        CatalogPullActionKind::PreserveAsCopyAndOverwrite { copy } => {
            preserve_and_overwrite(
                environment.registry,
                environment.repository,
                PreservationRequest {
                    storage_id: storage_id.clone(),
                    plan_id: plan_id.clone(),
                    action_id: action.action_id.clone(),
                    coordinate: coordinate.clone(),
                    resource_id: action.resource_id.clone(),
                    target: &action.target,
                    target_path: Path::new(target_path),
                    pin: copy,
                    incoming: &incoming,
                    now,
                },
            )?;
            claim(CatalogActionOutcome::PreservedAndUpdated)?;
            Ok(CatalogActionOutcome::PreservedAndUpdated)
        }
        CatalogPullActionKind::Install
        | CatalogPullActionKind::Update
        | CatalogPullActionKind::OverwriteWithStorage => {
            replace_directory_target(
                environment.registry,
                environment.repository,
                DirectoryReplaceRequest {
                    storage_id: storage_id.clone(),
                    plan_id: plan_id.clone(),
                    action_id: action.action_id.clone(),
                    coordinate: coordinate.clone(),
                    target: &action.target,
                    target_path: Path::new(target_path),
                    expected: &action.expected_target_state,
                    incoming: &incoming,
                    now,
                },
            )?;
            let outcome = match &action.expected_target_state {
                CatalogTargetExpectation::Absent => CatalogActionOutcome::Installed,
                _ => CatalogActionOutcome::Updated,
            };
            claim(outcome)?;
            Ok(outcome)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn record_claim(
    repository: &CatalogStateRepository,
    storage_id: &StorageId,
    coordinate: &CatalogCoordinate,
    target: &CatalogTargetConfig,
    resource_id: &CatalogResourceId,
    content_sha256: &str,
    outcome: CatalogActionOutcome,
    plan_id: &CatalogPlanId,
    now: u64,
) -> Result<(), String> {
    repository.mutate_materializations(|document| {
        document
            .claims
            .retain(|claim| !(claim.target == *target && &claim.storage_id == storage_id));
        document.claims.push(CatalogClaim {
            target: target.clone(),
            coordinate: coordinate.clone(),
            storage_id: storage_id.clone(),
            resource_id: resource_id.clone(),
            content_sha256: content_sha256.to_string(),
            outcome,
            recorded_at: now,
            plan_id: Some(plan_id.clone()),
        });
        Ok(())
    })
}

struct RecoveryPaths {
    target: PathBuf,
    copy: Option<PathBuf>,
    backup: Option<PathBuf>,
    root_class: String,
}

fn recovery_paths(
    repository: &CatalogStateRepository,
    entry: &CatalogOperationJournalEntry,
) -> Result<RecoveryPaths, String> {
    let identity = entry.target.decode()?;
    let target = Path::new(&identity.canonical_root).join(&identity.member);
    if entry.target_path.as_deref().map(Path::new) != Some(target.as_path()) {
        return Err("catalog recovery target path does not match its identity".to_string());
    }

    let copy = match (&entry.copy_target_path, &entry.copy_name) {
        (Some(path), Some(name)) => {
            let expected = Path::new(&identity.canonical_root).join(name);
            if Path::new(path) != expected {
                return Err("catalog recovery copy path does not match its identity".to_string());
            }
            Some(expected)
        }
        (None, None) => None,
        _ => return Err("catalog recovery copy path is incomplete".to_string()),
    };

    let backup = match &entry.backup_path {
        Some(path) => {
            let target_name = target
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("catalog recovery target has no name")?;
            let plan_id = entry
                .plan_id
                .as_ref()
                .ok_or("catalog recovery backup has no plan")?;
            let repository_backup = repository
                .root()
                .join("backups")
                .join("catalog_resources")
                .join(plan_id.as_str())
                .join(format!("{}-{}", entry.operation_id, target_name));
            let sibling_backup = target
                .parent()
                .ok_or("catalog recovery target has no parent")?
                .join(format!(
                    ".mallard-backup-{}-{}",
                    entry.operation_id, target_name
                ));
            let path = PathBuf::from(path);
            if path != repository_backup && path != sibling_backup {
                return Err(
                    "catalog recovery backup path is not bound to its operation".to_string()
                );
            }
            Some(path)
        }
        None => None,
    };

    Ok(RecoveryPaths {
        target,
        copy,
        backup,
        root_class: identity.root_class,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecoveryObservation {
    Absent,
    Present(String),
    Unverifiable,
}

fn recovery_observation(
    registry: &CatalogRegistry,
    capability: Option<&Arc<CatalogCapability>>,
    root_class: &str,
    path: &Path,
) -> RecoveryObservation {
    let Some(capability) = capability else {
        return RecoveryObservation::Unverifiable;
    };
    match observed_digest(registry, capability, root_class, path) {
        Ok(None) => RecoveryObservation::Absent,
        Ok(Some(digest)) => RecoveryObservation::Present(digest),
        Err(_) => RecoveryObservation::Unverifiable,
    }
}

fn backup_observation(path: &Path) -> RecoveryObservation {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => RecoveryObservation::Absent,
        Err(_) => RecoveryObservation::Unverifiable,
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            match backup_tree_sha256(path) {
                Ok(digest) => RecoveryObservation::Present(digest),
                Err(_) => RecoveryObservation::Unverifiable,
            }
        }
        Ok(_) => RecoveryObservation::Unverifiable,
    }
}

/// Startup recovery verifies every journal binding before it touches a
/// resource. Native operations are never replayed. A missing target is
/// distinct from one this build cannot verify.
pub fn recover_catalog_operations(
    registry: &CatalogRegistry,
    repository: &CatalogStateRepository,
    now: u64,
) -> Result<Vec<String>, String> {
    let operations = repository.load_operations()?;
    let mut messages = Vec::new();
    for entry in operations.operations {
        if entry.phase.is_terminal() {
            continue;
        }
        match entry.kind {
            CatalogOperationKind::NativeApply => {
                // Never replay stale native arguments; fresh inventory decides.
                journal_phase(
                    repository,
                    &entry.operation_id,
                    CatalogOperationPhase::Failed,
                    Some(CatalogActionOutcome::Partial),
                    Some(
                        "a native operation was interrupted; Refresh to re-inventory the provider"
                            .to_string(),
                    ),
                    now,
                )?;
                messages.push(format!(
                    "A {} native operation was interrupted and needs a fresh review",
                    entry.target.provider
                ));
            }
            CatalogOperationKind::DirectoryReplace
            | CatalogOperationKind::Preservation
            | CatalogOperationKind::Fork => {
                let paths = match recovery_paths(repository, &entry) {
                    Ok(paths) => paths,
                    Err(_) => {
                        let message =
                            "interrupted; its recovery journal is invalid and was left unchanged";
                        journal_phase(
                            repository,
                            &entry.operation_id,
                            CatalogOperationPhase::Failed,
                            Some(CatalogActionOutcome::Failed),
                            Some(message.to_string()),
                            now,
                        )?;
                        messages.push(format!("A catalog operation was {message}"));
                        continue;
                    }
                };
                let capability = registry.capability(&entry.coordinate);
                let target_state = recovery_observation(
                    registry,
                    capability.as_ref(),
                    &paths.root_class,
                    &paths.target,
                );
                let copy_state = match &paths.copy {
                    Some(copy) => {
                        recovery_observation(registry, capability.as_ref(), &paths.root_class, copy)
                    }
                    None => RecoveryObservation::Absent,
                };
                let copy_ok = matches!(
                    (&copy_state, &entry.copy_content_sha256),
                    (RecoveryObservation::Present(observed), Some(expected)) if observed == expected
                );

                let completed = matches!(
                    (&target_state, &entry.incoming_content_sha256),
                    (RecoveryObservation::Present(observed), Some(expected)) if observed == expected
                );
                let mut restored = false;
                let mut unverifiable = matches!(target_state, RecoveryObservation::Unverifiable)
                    || paths.copy.is_some()
                        && matches!(copy_state, RecoveryObservation::Unverifiable);
                if !completed && matches!(target_state, RecoveryObservation::Absent) {
                    if let (Some(backup), Some(expected)) =
                        (&paths.backup, &entry.backup_tree_sha256)
                    {
                        match backup_observation(backup) {
                            RecoveryObservation::Present(observed) if observed == *expected => {
                                restored = fs::rename(backup, &paths.target).is_ok();
                            }
                            RecoveryObservation::Unverifiable => unverifiable = true,
                            RecoveryObservation::Absent | RecoveryObservation::Present(_) => {}
                        }
                    }
                }

                let phase = if restored {
                    CatalogOperationPhase::RolledBack
                } else {
                    CatalogOperationPhase::Failed
                };
                let message = if unverifiable {
                    "interrupted; its state could not be verified and was left unchanged"
                } else if restored {
                    "interrupted; the original was restored from its backup"
                } else if copy_ok {
                    "interrupted; the preserved copy is intact and will be reused on retry"
                } else {
                    "interrupted; review the catalog again"
                };
                journal_phase(
                    repository,
                    &entry.operation_id,
                    phase,
                    Some(CatalogActionOutcome::Failed),
                    Some(message.to_string()),
                    now,
                )?;
                messages.push(format!(
                    "A catalog operation on '{}' was {}",
                    entry.copy_name.as_deref().unwrap_or("a target"),
                    message
                ));
                if entry.kind == CatalogOperationKind::Preservation
                    && !copy_ok
                    && !matches!(copy_state, RecoveryObservation::Unverifiable)
                {
                    repository.mutate_preservations(|document| {
                        document
                            .witnesses
                            .retain(|witness| witness.operation_id != entry.operation_id);
                        Ok(())
                    })?;
                }
            }
        }
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::super::adapters::{CmdOutput, MarketplaceObservation, ProviderInventory};
    use super::super::domain::{
        CatalogOperationJournalEntry, CatalogOperationsDocument, CatalogTargetIdentity,
        CatalogTargetKey, ProviderKey, CATALOG_SCHEMA,
    };
    use super::super::plugin_intent::{
        GitMarketplaceSource, PluginInstallIntent, PluginInstallIntentDescriptor,
        PluginInstallIntentIdentity, PluginResolutionPolicy, PortablePluginSource,
    };
    use super::*;

    #[derive(Default)]
    struct ChangedMarketplaceRunner {
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl ChangedMarketplaceRunner {
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl PluginProcessRunner for ChangedMarketplaceRunner {
        fn run(
            &self,
            provider: &ProviderKey,
            _home: &Path,
            argv: &[String],
        ) -> Result<CmdOutput, String> {
            assert_eq!(provider.as_str(), "codex");
            self.calls.lock().unwrap().push(argv.to_vec());
            let stdout = match argv.join(" ").as_str() {
                "plugin marketplace list --json" => {
                    r#"{"marketplaces":[{"name":"acme","marketplaceSource":{"sourceType":"git","source":"https://example.com/acme/repository-b","ref":"release"}}]}"#
                }
                "plugin list --json" => r#"{"installed":[]}"#,
                command => return Err(format!("unexpected native invocation: {command}")),
            };
            Ok(CmdOutput {
                success: true,
                stdout: stdout.to_string(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn native_apply_rejects_changed_marketplace_before_journal_or_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        std::fs::create_dir(&home).unwrap();
        let home = home.canonicalize().unwrap();
        let repository = CatalogStateRepository::from_home_dir(temp.path()).unwrap();
        let provider = ProviderKey::parse("codex").unwrap();
        let resource = PluginInstallIntent {
            identity: PluginInstallIntentIdentity {
                namespace: "acme".to_string(),
                plugin_id: "tools@acme".to_string(),
            },
            descriptor: PluginInstallIntentDescriptor {
                source: PortablePluginSource::ProviderMarketplace {
                    namespace: "acme".to_string(),
                    git: Some(GitMarketplaceSource {
                        location: "https://example.com/acme/repository-a".to_string(),
                        git_ref: Some("release".to_string()),
                    }),
                },
                resolution: PluginResolutionPolicy::ProviderResolved,
            },
        }
        .envelope(&provider)
        .unwrap();
        let registry = CatalogRegistry::production();
        let capability = registry.require_capability(&resource.coordinate()).unwrap();
        let approved_inventory = ProviderInventory {
            marketplaces: vec![MarketplaceObservation {
                name: "acme".to_string(),
                source_type: "git".to_string(),
                source: "https://example.com/acme/repository-a".to_string(),
                git_ref: Some("release".to_string()),
                root: None,
            }],
            ..Default::default()
        };
        let native = capability
            .adapter
            .plan_native_apply(
                &resource,
                &approved_inventory,
                PluginApplyMode::InstallMissing,
            )
            .unwrap();
        let target = CatalogTargetConfig::new(
            provider,
            CatalogTargetKey::parse("user.plugins").unwrap(),
            &CatalogTargetIdentity {
                canonical_root: home.to_string_lossy().to_string(),
                root_class: "user".to_string(),
                member: "tools@acme".to_string(),
            },
        )
        .unwrap();
        let action = CatalogPullAction {
            action_id: CatalogPlanActionId::parse("action-000000000000000000000000").unwrap(),
            resource_id: resource.resource_id.clone(),
            kind: CatalogPullActionKind::Install,
            requires_destructive_approval: false,
            target,
            target_path: None,
            native: Some(native.clone()),
            expected_target_state: CatalogTargetExpectation::Absent,
            incoming_content_sha256: resource.content_sha256.clone(),
        };
        action.validate().unwrap();
        let runner = ChangedMarketplaceRunner::default();

        let error = execute_native_action(
            &repository,
            &runner,
            capability.as_ref(),
            &StorageId::parse("storage-a").unwrap(),
            &CatalogPlanId::parse("catplan-000000000000000000000000").unwrap(),
            &action,
            &native,
            &resource,
            &home,
            1,
        )
        .unwrap_err();

        assert!(error.contains("no longer matches the approved source and ref"));
        let calls = runner.calls();
        assert_eq!(
            calls.iter().map(|argv| argv.join(" ")).collect::<Vec<_>>(),
            vec!["plugin marketplace list --json", "plugin list --json"]
        );
        assert!(repository.load_operations().unwrap().operations.is_empty());
        assert!(repository
            .load_materializations()
            .unwrap()
            .claims
            .is_empty());
    }

    fn interrupted_replace(root: &Path, backup_path: &Path) -> CatalogOperationJournalEntry {
        let operation_id = CatalogOperationId::parse("catop-000000000000000000000000").unwrap();
        let target = CatalogTargetConfig::new(
            ProviderKey::parse("future").unwrap(),
            CatalogTargetKey::parse("user.plugins").unwrap(),
            &CatalogTargetIdentity {
                canonical_root: root.to_string_lossy().to_string(),
                root_class: "user".to_string(),
                member: "tools".to_string(),
            },
        )
        .unwrap();
        CatalogOperationJournalEntry {
            operation_id,
            kind: CatalogOperationKind::DirectoryReplace,
            coordinate: CatalogCoordinate::new("future", "plugin", "future.plugin", 1).unwrap(),
            storage_id: Some(StorageId::parse("storage-a").unwrap()),
            plan_id: Some(CatalogPlanId::parse("catplan-000000000000000000000000").unwrap()),
            action_id: Some(CatalogPlanActionId::parse("action-000000000000000000000000").unwrap()),
            target,
            target_path: Some(root.join("tools").to_string_lossy().to_string()),
            backup_path: Some(backup_path.to_string_lossy().to_string()),
            backup_tree_sha256: Some("c".repeat(64)),
            copy_target_path: None,
            copy_name: None,
            copy_resource_id: None,
            expected_target_sha256: Some("a".repeat(64)),
            incoming_content_sha256: Some("b".repeat(64)),
            copy_content_sha256: None,
            phase: CatalogOperationPhase::OriginalBackedUp,
            outcome: None,
            message: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn recovery_distinguishes_unverifiable_from_absent_and_verifies_the_backup_digest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        std::fs::create_dir_all(&root).unwrap();
        let operation_id = CatalogOperationId::parse("catop-000000000000000000000000").unwrap();
        let backup = root.join(format!(".mallard-backup-{operation_id}-tools"));
        std::fs::create_dir(&backup).unwrap();
        let repository = CatalogStateRepository::from_home_dir(temp.path()).unwrap();
        repository
            .mutate_operations(|document| {
                document
                    .operations
                    .push(interrupted_replace(&root, &backup));
                Ok(())
            })
            .unwrap();

        recover_catalog_operations(CatalogRegistry::production().as_ref(), &repository, 2).unwrap();

        assert!(backup.is_dir());
        assert!(!root.join("tools").exists());
        assert_eq!(
            repository.load_operations().unwrap().operations[0].phase,
            CatalogOperationPhase::Failed
        );

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        let operation_id = CatalogOperationId::parse("catop-000000000000000000000000").unwrap();
        let backup = root.join(format!(".mallard-backup-{operation_id}-tools"));
        std::fs::create_dir(&backup).unwrap();
        let mut entry = interrupted_replace(&root, &backup);
        entry.coordinate =
            CatalogCoordinate::new("codex", "agent_skill", "agentskills.io.skill", 1).unwrap();
        entry.target = CatalogTargetConfig::new(
            ProviderKey::parse("codex").unwrap(),
            CatalogTargetKey::parse("provider-home.skills").unwrap(),
            &CatalogTargetIdentity {
                canonical_root: root.to_string_lossy().to_string(),
                root_class: "provider-home".to_string(),
                member: "tools".to_string(),
            },
        )
        .unwrap();
        let repository = CatalogStateRepository::from_home_dir(temp.path()).unwrap();
        repository
            .mutate_operations(|document| {
                document.operations.push(entry);
                Ok(())
            })
            .unwrap();

        recover_catalog_operations(CatalogRegistry::production().as_ref(), &repository, 2).unwrap();

        assert!(backup.is_dir());
        assert!(!root.join("tools").exists());

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        let operation_id = CatalogOperationId::parse("catop-000000000000000000000000").unwrap();
        let backup = root.join(format!(".mallard-backup-{operation_id}-tools"));
        std::fs::create_dir(&backup).unwrap();
        let mut entry = interrupted_replace(&root, &backup);
        entry.coordinate =
            CatalogCoordinate::new("codex", "agent_skill", "agentskills.io.skill", 1).unwrap();
        entry.target = CatalogTargetConfig::new(
            ProviderKey::parse("codex").unwrap(),
            CatalogTargetKey::parse("provider-home.skills").unwrap(),
            &CatalogTargetIdentity {
                canonical_root: root.to_string_lossy().to_string(),
                root_class: "provider-home".to_string(),
                member: "tools".to_string(),
            },
        )
        .unwrap();
        entry.backup_tree_sha256 = Some(backup_tree_sha256(&backup).unwrap());
        let repository = CatalogStateRepository::from_home_dir(temp.path()).unwrap();
        repository
            .mutate_operations(|document| {
                document.operations.push(entry);
                Ok(())
            })
            .unwrap();

        recover_catalog_operations(CatalogRegistry::production().as_ref(), &repository, 2).unwrap();

        assert!(!backup.exists());
        assert!(root.join("tools").is_dir());
    }

    #[test]
    fn operation_journal_rejects_unbound_paths_digests_and_kind_phases() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        std::fs::create_dir_all(&root).unwrap();
        let operation_id = CatalogOperationId::parse("catop-000000000000000000000000").unwrap();
        let valid_backup = root.join(format!(".mallard-backup-{operation_id}-tools"));
        let entry = interrupted_replace(&root, &valid_backup);

        let mut invalid_entries = Vec::new();
        let mut bad_path = entry.clone();
        bad_path.backup_path = Some(
            temp.path()
                .join("unrelated")
                .join(format!("{operation_id}-tools"))
                .to_string_lossy()
                .to_string(),
        );
        invalid_entries.push(bad_path);
        let mut bad_digest = entry.clone();
        bad_digest.incoming_content_sha256 = Some("bad".to_string());
        invalid_entries.push(bad_digest);
        let mut bad_phase = entry.clone();
        bad_phase.phase = CatalogOperationPhase::CopyActivated;
        invalid_entries.push(bad_phase);
        let mut bad_kind = entry;
        bad_kind.kind = CatalogOperationKind::NativeApply;
        invalid_entries.push(bad_kind);

        for invalid in invalid_entries {
            let document = CatalogOperationsDocument {
                schema: CATALOG_SCHEMA,
                revision: 0,
                operations: vec![invalid],
            };
            assert!(document.validate().is_err());
        }

        let forged_backup = temp
            .path()
            .join("foreign")
            .join("backups")
            .join("catalog_resources")
            .join("catplan-000000000000000000000000")
            .join(format!("{operation_id}-tools"));
        std::fs::create_dir_all(&forged_backup).unwrap();
        let forged = interrupted_replace(&root, &forged_backup);
        CatalogOperationsDocument {
            schema: CATALOG_SCHEMA,
            revision: 0,
            operations: vec![forged.clone()],
        }
        .validate()
        .unwrap();
        let repository =
            CatalogStateRepository::from_home_dir(temp.path().join("state-home")).unwrap();
        repository
            .mutate_operations(|document| {
                document.operations.push(forged);
                Ok(())
            })
            .unwrap();

        recover_catalog_operations(CatalogRegistry::production().as_ref(), &repository, 2).unwrap();

        assert!(forged_backup.is_dir());
        assert!(!root.join("tools").exists());
    }
}
