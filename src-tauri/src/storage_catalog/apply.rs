//! Approved catalog mutation: target-safe directory replacement, the
//! composite preservation transaction, the standalone Fork copy, the native
//! plugin executor, and startup recovery.
//!
//! Every skill/pet mutation is journaled before it changes the target,
//! staged on the target filesystem, digest-verified, backed up, activated by
//! rename with no-clobber claims, verified again, and recoverable from the
//! same journal. Recovery resumes the same operation and name; a retry never
//! mints a second copy.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use mallard_domain::StorageId;

use super::adapters::{adapter_for, redact, PluginProcessRunner};
use super::codex_pet::{
    capture_pet, materialize_pet, observed_pet_digest, pet_resource_from_capture, renamed_pet,
};
use super::domain::{
    CatalogActionOutcome, CatalogActionResult, CatalogClaim, CatalogOperationId,
    CatalogOperationJournalEntry, CatalogOperationKind, CatalogOperationPhase, CatalogPlanActionId,
    CatalogPlanId, CatalogPreservationPin, CatalogPreservationWitness, CatalogPullAction,
    CatalogPullActionKind, CatalogPullDisposition, CatalogResource, CatalogResourceForkPlan,
    CatalogResourceId, CatalogResourcePullPlan, CatalogTargetExpectation, CodexPetDescriptor,
    CodexPetFormat, LocalCatalogTargetId, NativePluginPlan, PluginApplyMode,
};
use super::persistence::CatalogStateRepository;
use super::skill_tree::{
    capture_skill_tree, materialize_skill_tree, rename_skill_tree, skill_resource_from_tree,
    verify_tree_matches, CapturedSkillFile,
};

/// The storage version of one directory resource, fully fetched and verified
/// before any local mutation.
pub enum IncomingDirectoryPayload {
    Skill {
        scope: super::domain::AgentSkillScope,
        effective_name: String,
        install_directory: String,
        files: BTreeMap<String, CapturedSkillFile>,
    },
    Pet {
        format: CodexPetFormat,
        descriptor: CodexPetDescriptor,
        atlas: Vec<u8>,
    },
}

impl IncomingDirectoryPayload {
    fn content_sha256(&self) -> Result<String, String> {
        match self {
            IncomingDirectoryPayload::Skill {
                scope,
                effective_name,
                install_directory,
                files,
            } => Ok(
                skill_resource_from_tree(*scope, effective_name, install_directory, files)?
                    .content_sha256,
            ),
            IncomingDirectoryPayload::Pet {
                format,
                descriptor,
                atlas,
            } => {
                let captured = super::codex_pet::CapturedPet {
                    format: *format,
                    pet_id: descriptor.pet_id.clone(),
                    display_name: descriptor.display_name.clone(),
                    description: descriptor.description.clone(),
                    atlas_basename: descriptor.atlas_basename.clone(),
                    atlas_bytes: atlas.clone(),
                    extra_files: Vec::new(),
                };
                Ok(pet_resource_from_capture(&captured)?.content_sha256)
            }
        }
    }

    fn materialize(&self, stage: &Path) -> Result<(), String> {
        match self {
            IncomingDirectoryPayload::Skill { files, .. } => materialize_skill_tree(stage, files),
            IncomingDirectoryPayload::Pet {
                format,
                descriptor,
                atlas,
            } => materialize_pet(stage, *format, descriptor, atlas),
        }
    }

    fn verify_at(&self, target: &Path) -> Result<(), String> {
        match self {
            IncomingDirectoryPayload::Skill { files, .. } => verify_tree_matches(target, files),
            IncomingDirectoryPayload::Pet { format, .. } => {
                let (observed_format, _) = observed_pet_digest(target)?;
                if observed_format != *format {
                    return Err("the activated pet does not match its format".to_string());
                }
                Ok(())
            }
        }
    }
}

/// Observed digest of a directory target, or `None` when absent.
pub fn observed_directory_digest(
    target: &LocalCatalogTargetId,
    path: &Path,
) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("inspect '{}': {}", path.display(), error)),
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            return Err(format!("'{}' is not a real directory", path.display()))
        }
        Ok(_) => {}
    }
    match target {
        LocalCatalogTargetId::SkillDirectory { scope, .. } => {
            let tree = capture_skill_tree(path)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("skill directory name is not UTF-8")?;
            let effective = tree
                .declared_name
                .clone()
                .unwrap_or_else(|| name.to_string());
            Ok(Some(
                skill_resource_from_tree(*scope, &effective, name, &tree.files)?.content_sha256,
            ))
        }
        LocalCatalogTargetId::PetDirectory { .. } => {
            let (_, digest) = observed_pet_digest(path)?;
            Ok(Some(digest))
        }
        LocalCatalogTargetId::Plugin { .. } => Err("plugins have no directory target".to_string()),
    }
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
) -> Result<PathBuf, String> {
    let name = target_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("backup target has no name")?;
    let preferred = repository
        .backup_dir(plan_id)?
        .join(format!("{}-{}", operation_id, name));
    match fs::rename(target_path, &preferred) {
        Ok(()) => Ok(preferred),
        Err(_) => {
            // Cross-filesystem: keep the backup beside the target as a
            // hidden, journaled sibling instead of a recursive delete.
            let sibling = target_path
                .parent()
                .ok_or("backup target has no parent")?
                .join(format!(".mallard-backup-{}-{}", operation_id, name));
            fs::rename(target_path, &sibling)
                .map_err(|error| format!("back up '{}': {}", target_path.display(), error))?;
            Ok(sibling)
        }
    }
}

struct DirectoryReplaceRequest<'a> {
    storage_id: StorageId,
    plan_id: CatalogPlanId,
    action_id: CatalogPlanActionId,
    target: &'a LocalCatalogTargetId,
    target_path: &'a Path,
    expected: &'a CatalogTargetExpectation,
    incoming: &'a IncomingDirectoryPayload,
    now: u64,
}

/// One whole-directory replacement transaction. There is no
/// per-file merge.
fn replace_directory_target(
    repository: &CatalogStateRepository,
    request: DirectoryReplaceRequest<'_>,
) -> Result<(), String> {
    let root = request
        .target_path
        .parent()
        .ok_or("directory target has no parent")?;
    let _target_lock = repository.acquire_target_lock(root)?;

    let observed = observed_directory_digest(request.target, request.target_path)?;
    expectation_matches(request.expected, observed.as_deref())?;
    let incoming_digest = request.incoming.content_sha256()?;

    let operation_id = CatalogOperationId::generate()?;
    let target_name = request
        .target_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("directory target has no name")?;
    let stage_parent = stage_dir_for(root, &operation_id, "");
    let stage = match staged_child(&stage_parent, target_name) {
        Ok(stage) => stage,
        Err(error) => {
            let _ = fs::remove_dir_all(&stage_parent);
            return Err(error);
        }
    };
    let staged = request.incoming.materialize(&stage);
    if let Err(error) = staged {
        let _ = fs::remove_dir_all(&stage_parent);
        return Err(error);
    }
    if let Err(error) = request.incoming.verify_at(&stage) {
        let _ = fs::remove_dir_all(&stage_parent);
        return Err(error);
    }

    journal_upsert(
        repository,
        CatalogOperationJournalEntry {
            operation_id: operation_id.clone(),
            kind: CatalogOperationKind::DirectoryReplace,
            storage_id: Some(request.storage_id.clone()),
            plan_id: Some(request.plan_id.clone()),
            action_id: Some(request.action_id.clone()),
            target: request.target.clone(),
            target_path: Some(request.target_path.to_string_lossy().to_string()),
            backup_path: None,
            copy_target_path: None,
            copy_name: None,
            copy_resource_id: None,
            expected_target_sha256: observed.clone(),
            incoming_content_sha256: Some(incoming_digest.clone()),
            copy_content_sha256: None,
            phase: CatalogOperationPhase::Journaled,
            outcome: None,
            message: None,
            created_at: request.now,
            updated_at: request.now,
        },
    )?;

    let backup = if observed.is_some() {
        let backup = backup_target(
            repository,
            &request.plan_id,
            &operation_id,
            request.target_path,
        )?;
        repository.mutate_operations(|document| {
            if let Some(entry) = document
                .operations
                .iter_mut()
                .find(|entry| entry.operation_id == operation_id)
            {
                entry.backup_path = Some(backup.to_string_lossy().to_string());
                entry.phase = CatalogOperationPhase::OriginalBackedUp;
                entry.updated_at = request.now;
            }
            Ok(())
        })?;
        Some(backup)
    } else {
        None
    };

    let activation = activate_stage_no_clobber(&stage, request.target_path);
    let _ = fs::remove_dir_all(&stage_parent);
    if let Err(error) = activation {
        // Roll the backup into place when safe; the stage is discarded.
        if let Some(backup) = &backup {
            if fs::symlink_metadata(request.target_path).is_err() {
                let _ = fs::rename(backup, request.target_path);
            }
        }
        journal_phase(
            repository,
            &operation_id,
            CatalogOperationPhase::RolledBack,
            Some(CatalogActionOutcome::Failed),
            Some(error.clone()),
            request.now,
        )?;
        return Err(error);
    }

    if let Err(error) = request.incoming.verify_at(request.target_path) {
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
    resource_id: CatalogResourceId,
    target: &'a LocalCatalogTargetId,
    target_path: &'a Path,
    pin: &'a CatalogPreservationPin,
    incoming: &'a IncomingDirectoryPayload,
    now: u64,
}

/// The composite preservation transaction: create the
/// validated named copy, witness it, then back up and overwrite the original.
fn preserve_and_overwrite(
    repository: &CatalogStateRepository,
    request: PreservationRequest<'_>,
) -> Result<(), String> {
    let root = request
        .target_path
        .parent()
        .ok_or("preservation target has no parent")?;
    let _target_lock = repository.acquire_target_lock(root)?;

    // 1. Revalidate every pin.
    let observed = observed_directory_digest(request.target, request.target_path)?
        .ok_or("the diverged local version disappeared before preservation")?;
    if observed != request.pin.original_content_sha256 {
        return Err(
            "the local version changed after review; refresh the catalog and review again"
                .to_string(),
        );
    }
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
        && observed_directory_digest(request.target, &copy_target)
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
    let cleanup = |left: &Path, right: &Path| {
        let _ = fs::remove_dir_all(left);
        let _ = fs::remove_dir_all(right);
    };
    let copy_stage = if witness_valid {
        copy_stage_parent.join(&request.pin.copy_name)
    } else {
        let copy_stage = staged_child(&copy_stage_parent, &request.pin.copy_name)
            .inspect_err(|_| cleanup(&copy_stage_parent, &incoming_stage_parent))?;
        let transformed = build_transformed_copy(request.target, request.target_path, request.pin)?;
        if let Err(error) = transformed.materialize(&copy_stage) {
            cleanup(&copy_stage_parent, &incoming_stage_parent);
            return Err(error);
        }
        let copy_target_id = preserved_target_for(request.target, &request.pin.copy_name);
        let staged_digest = observed_directory_digest(&copy_target_id, &copy_stage)?;
        if staged_digest.as_deref() != Some(request.pin.transformed_content_sha256.as_str()) {
            cleanup(&copy_stage_parent, &incoming_stage_parent);
            return Err("the transformed copy failed digest validation".to_string());
        }
        copy_stage
    };
    let incoming_stage = staged_child(&incoming_stage_parent, target_name)
        .inspect_err(|_| cleanup(&copy_stage_parent, &incoming_stage_parent))?;
    if let Err(error) = request.incoming.materialize(&incoming_stage) {
        cleanup(&copy_stage_parent, &incoming_stage_parent);
        return Err(error);
    }
    if let Err(error) = request.incoming.verify_at(&incoming_stage) {
        cleanup(&copy_stage_parent, &incoming_stage_parent);
        return Err(error);
    }

    // 5. Journal the pinned operation and chosen name.
    journal_upsert(
        repository,
        CatalogOperationJournalEntry {
            operation_id: operation_id.clone(),
            kind: CatalogOperationKind::Preservation,
            storage_id: Some(request.storage_id.clone()),
            plan_id: Some(request.plan_id.clone()),
            action_id: Some(request.action_id.clone()),
            target: request.target.clone(),
            target_path: Some(request.target_path.to_string_lossy().to_string()),
            backup_path: None,
            copy_target_path: Some(request.pin.copy_target_path.clone()),
            copy_name: Some(request.pin.copy_name.clone()),
            copy_resource_id: Some(request.pin.copy_resource_id.clone()),
            expected_target_sha256: Some(request.pin.original_content_sha256.clone()),
            incoming_content_sha256: Some(request.pin.storage_content_sha256.clone()),
            copy_content_sha256: Some(request.pin.transformed_content_sha256.clone()),
            phase: CatalogOperationPhase::Journaled,
            outcome: None,
            message: None,
            created_at: request.now,
            updated_at: request.now,
        },
    )?;

    // 6. Activate the copy with no-clobber semantics and verify it.
    if !witness_valid {
        if let Err(error) = activate_stage_no_clobber(&copy_stage, &copy_target) {
            cleanup(&copy_stage_parent, &incoming_stage_parent);
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
        let copy_target_id = preserved_target_for(request.target, &request.pin.copy_name);
        let activated = observed_directory_digest(&copy_target_id, &copy_target)?;
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
    repository.mutate_preservations(|document| {
        document.witnesses.retain(|witness| {
            !(witness.source_target == *request.target
                && witness.storage_source_sha256 == request.pin.storage_content_sha256)
        });
        document.witnesses.push(CatalogPreservationWitness {
            storage_id: request.storage_id.clone(),
            source_resource_id: request.resource_id.clone(),
            source_target: request.target.clone(),
            local_source_sha256: request.pin.original_content_sha256.clone(),
            storage_source_sha256: request.pin.storage_content_sha256.clone(),
            preserved_resource_id: request.pin.copy_resource_id.clone(),
            preserved_target: preserved_target_for(request.target, &request.pin.copy_name),
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
    )?;
    repository.mutate_operations(|document| {
        if let Some(entry) = document
            .operations
            .iter_mut()
            .find(|entry| entry.operation_id == operation_id)
        {
            entry.backup_path = Some(backup.to_string_lossy().to_string());
            entry.phase = CatalogOperationPhase::OriginalBackedUp;
            entry.updated_at = request.now;
        }
        Ok(())
    })?;
    let activation = activate_stage_no_clobber(&incoming_stage, request.target_path);
    let _ = fs::remove_dir_all(&incoming_stage_parent);
    if let Err(error) = activation {
        // The verified copy stays; restore the original from backup.
        if fs::symlink_metadata(request.target_path).is_err() {
            let _ = fs::rename(&backup, request.target_path);
        }
        journal_phase(
            repository,
            &operation_id,
            CatalogOperationPhase::RolledBack,
            Some(CatalogActionOutcome::Failed),
            Some(error.clone()),
            request.now,
        )?;
        return Err(error);
    }
    if let Err(error) = request.incoming.verify_at(request.target_path) {
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

fn preserved_target_for(source: &LocalCatalogTargetId, copy_name: &str) -> LocalCatalogTargetId {
    match source {
        LocalCatalogTargetId::SkillDirectory {
            scope,
            canonical_root,
            ..
        } => LocalCatalogTargetId::SkillDirectory {
            scope: *scope,
            canonical_root: canonical_root.clone(),
            install_directory: copy_name.to_string(),
        },
        LocalCatalogTargetId::PetDirectory { canonical_home, .. } => {
            LocalCatalogTargetId::PetDirectory {
                canonical_home: canonical_home.clone(),
                pet_id: copy_name.to_string(),
            }
        }
        LocalCatalogTargetId::Plugin { .. } => source.clone(),
    }
}

/// Build the transformed local copy payload from the current on-disk source.
fn build_transformed_copy(
    target: &LocalCatalogTargetId,
    source_path: &Path,
    pin: &CatalogPreservationPin,
) -> Result<IncomingDirectoryPayload, String> {
    match target {
        LocalCatalogTargetId::SkillDirectory { scope, .. } => {
            let tree = capture_skill_tree(source_path)?;
            let files = rename_skill_tree(&tree.files, &pin.copy_name)?;
            Ok(IncomingDirectoryPayload::Skill {
                scope: *scope,
                effective_name: pin.copy_name.clone(),
                install_directory: pin.copy_name.clone(),
                files,
            })
        }
        LocalCatalogTargetId::PetDirectory { .. } => {
            let captured = capture_pet(source_path)?;
            let renamed = renamed_pet(&captured, &pin.copy_name)?;
            let resource = pet_resource_from_capture(&renamed)?;
            Ok(IncomingDirectoryPayload::Pet {
                format: renamed.format,
                descriptor: resource.descriptor,
                atlas: renamed.atlas_bytes,
            })
        }
        LocalCatalogTargetId::Plugin { .. } => {
            Err("plugins never use preservation copies".to_string())
        }
    }
}

/// Apply the standalone resource Fork plan: the same semantic rename,
/// staging, no-clobber activation, verification, journal, and recovery
/// primitive as Pull preservation, ending after the verified copy. It never
/// overwrites another target, never records a Pull preservation witness, and
/// never invokes any thread command or native thread-fork executor.
pub fn apply_resource_fork(
    repository: &CatalogStateRepository,
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
    let _target_lock = repository.acquire_target_lock(root)?;

    let observed = observed_directory_digest(&plan.source_target, &source_path)?
        .ok_or("the fork source disappeared; refresh and review again")?;
    if observed != plan.source_content_sha256 {
        return Err("the fork source changed after review; review it again".to_string());
    }

    let operation_id = CatalogOperationId::generate()?;
    let stage_parent = stage_dir_for(root, &operation_id, "-fork");
    let stage = staged_child(&stage_parent, &plan.copy_name)?;
    let pin = CatalogPreservationPin {
        copy_name: plan.copy_name.clone(),
        copy_resource_id: plan.copy_resource_id.clone(),
        copy_target_path: plan.copy_target_path.clone(),
        transformed_content_sha256: plan.transformed_content_sha256.clone(),
        original_content_sha256: plan.source_content_sha256.clone(),
        storage_content_sha256: plan.source_content_sha256.clone(),
        reusable_witness: false,
    };
    let payload = match build_transformed_copy(&plan.source_target, &source_path, &pin) {
        Ok(payload) => payload,
        Err(error) => {
            let _ = fs::remove_dir_all(&stage_parent);
            return Err(error);
        }
    };
    if let Err(error) = payload.materialize(&stage) {
        let _ = fs::remove_dir_all(&stage_parent);
        return Err(error);
    }
    let staged_digest = observed_directory_digest(
        &preserved_target_for(&plan.source_target, &plan.copy_name),
        &stage,
    )?;
    if staged_digest.as_deref() != Some(plan.transformed_content_sha256.as_str()) {
        let _ = fs::remove_dir_all(&stage_parent);
        return Err("the fork copy failed digest validation".to_string());
    }

    journal_upsert(
        repository,
        CatalogOperationJournalEntry {
            operation_id: operation_id.clone(),
            kind: CatalogOperationKind::Fork,
            storage_id: None,
            plan_id: Some(plan.plan_id.clone()),
            action_id: None,
            target: plan.source_target.clone(),
            target_path: Some(plan.source_path.clone()),
            backup_path: None,
            copy_target_path: Some(plan.copy_target_path.clone()),
            copy_name: Some(plan.copy_name.clone()),
            copy_resource_id: Some(plan.copy_resource_id.clone()),
            expected_target_sha256: Some(plan.source_content_sha256.clone()),
            incoming_content_sha256: None,
            copy_content_sha256: Some(plan.transformed_content_sha256.clone()),
            phase: CatalogOperationPhase::Journaled,
            outcome: None,
            message: None,
            created_at: now,
            updated_at: now,
        },
    )?;

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
    let verify_target = preserved_target_for(&plan.source_target, &plan.copy_name);
    let activated = observed_directory_digest(&verify_target, &copy_target)?;
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

/// Execute one approved native plugin operation and verify the result. A
/// clean exit with failed verification is `partial`, never success.
#[allow(clippy::too_many_arguments)]
fn execute_plugin_action(
    repository: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    storage_id: &StorageId,
    plan_id: &CatalogPlanId,
    action: &CatalogPullAction,
    native: &NativePluginPlan,
    intent: &super::domain::PluginInstallIntentDescriptor,
    home: &Path,
    now: u64,
) -> Result<CatalogActionOutcome, String> {
    let _target_lock = repository.acquire_target_lock(home)?;
    let operation_id = CatalogOperationId::generate()?;
    journal_upsert(
        repository,
        CatalogOperationJournalEntry {
            operation_id: operation_id.clone(),
            kind: CatalogOperationKind::PluginApply,
            storage_id: Some(storage_id.clone()),
            plan_id: Some(plan_id.clone()),
            action_id: Some(action.action_id.clone()),
            target: action.target.clone(),
            target_path: None,
            backup_path: None,
            copy_target_path: None,
            copy_name: None,
            copy_resource_id: None,
            expected_target_sha256: None,
            incoming_content_sha256: Some(action.incoming_content_sha256.clone()),
            copy_content_sha256: None,
            phase: CatalogOperationPhase::Journaled,
            outcome: None,
            message: None,
            created_at: now,
            updated_at: now,
        },
    )?;

    let adapter = adapter_for(native.provider);
    // Pre-launch recheck through fresh native inventory. The reviewed local
    // semantic intent (or reviewed absence) must still be exact before a
    // provider command can launch.
    let current = adapter
        .discover_plugins(home, runner)
        .map_err(|blocker| blocker.to_string())?;
    let current_digest = current
        .observation(&intent.plugin_id)
        .filter(|observation| observation.installed)
        .and_then(|observation| adapter.portable_intent(observation, &current))
        .map(|resource| resource.content_sha256);
    if let Err(error) =
        expectation_matches(&action.expected_target_state, current_digest.as_deref())
    {
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

    let mut argv_sets: Vec<&Vec<String>> = native.marketplace_argv.iter().collect();
    argv_sets.push(&native.argv);
    for argv in argv_sets {
        let output = runner
            .run(native.provider, home, argv)
            .inspect_err(|error| {
                let _ = journal_phase(
                    repository,
                    &operation_id,
                    CatalogOperationPhase::Failed,
                    Some(CatalogActionOutcome::Failed),
                    Some(error.clone()),
                    now,
                );
            })?;
        if !output.success {
            let detail = redact(output.stderr.trim());
            let message = format!(
                "{} exited unsuccessfully: {}",
                native.provider.as_str(),
                if detail.is_empty() {
                    redact(output.stdout.trim())
                } else {
                    detail
                }
            );
            journal_phase(
                repository,
                &operation_id,
                CatalogOperationPhase::Failed,
                Some(CatalogActionOutcome::Failed),
                Some(message.clone()),
                now,
            )?;
            return Err(message);
        }
    }

    // Post-exit re-inventory: the plugin must be present AND the resulting
    // semantic intent must match what was approved. A clean exit that leaves
    // a different intent installed is partial, never success.
    let verification = adapter
        .verify_plugin_apply(intent, home, runner)
        .map_err(|blocker| blocker.to_string())?;
    let resulting_intent_matches = if verification.verified {
        let fresh = adapter
            .discover_plugins(home, runner)
            .map_err(|blocker| blocker.to_string())?;
        verification
            .observation
            .as_ref()
            .and_then(|observation| adapter.portable_intent(observation, &fresh))
            .is_some_and(|rebuilt| rebuilt.content_sha256 == action.incoming_content_sha256)
    } else {
        false
    };
    if verification.verified && resulting_intent_matches {
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
    } else {
        let detail = if verification.verified {
            format!(
                "{} reports the plugin installed, but its install intent does not match the                  approved catalog intent yet",
                native.provider.as_str()
            )
        } else {
            verification.detail.clone()
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

/// Fetch one verified payload object by digest and exact size.
pub type PayloadFetcher<'a> = dyn Fn(&str, u64) -> Result<Vec<u8>, String> + 'a;

pub struct PullApplyEnvironment<'a> {
    pub repository: &'a CatalogStateRepository,
    pub runner: &'a dyn PluginProcessRunner,
    pub fetch_payload: &'a PayloadFetcher<'a>,
}

fn incoming_payload_for(
    resource: &CatalogResource,
    fetch: &PayloadFetcher<'_>,
) -> Result<IncomingDirectoryPayload, String> {
    match resource {
        CatalogResource::AgentSkill(skill) => {
            let mut files = BTreeMap::new();
            for (path, entry) in &skill.files {
                let bytes = fetch(&entry.sha256, entry.size)?;
                files.insert(
                    path.clone(),
                    CapturedSkillFile {
                        bytes,
                        mode: entry.mode,
                    },
                );
            }
            Ok(IncomingDirectoryPayload::Skill {
                scope: skill.descriptor.scope,
                effective_name: skill.descriptor.effective_name.clone(),
                install_directory: skill.descriptor.install_directory.clone(),
                files,
            })
        }
        CatalogResource::CodexPetV1(pet) => Ok(IncomingDirectoryPayload::Pet {
            format: CodexPetFormat::V1,
            descriptor: pet.descriptor.clone(),
            atlas: fetch(&pet.descriptor.atlas.sha256, pet.descriptor.atlas.size)?,
        }),
        CatalogResource::CodexPetV2(pet) => Ok(IncomingDirectoryPayload::Pet {
            format: CodexPetFormat::V2,
            descriptor: pet.descriptor.clone(),
            atlas: fetch(&pet.descriptor.atlas.sha256, pet.descriptor.atlas.size)?,
        }),
        CatalogResource::PluginInstallIntent(_) => {
            Err("plugins own no payload directory".to_string())
        }
    }
}

/// Execute the single approved action (or explicit Skip) for a draft-linked
/// resource Pull. The command layer advances the accepted remote head only
/// after this returns a non-failed, non-partial outcome; explicit Skip is a
/// successful accept-without-mutation decision.
pub fn execute_resource_pull_action(
    environment: &PullApplyEnvironment<'_>,
    plan: &CatalogResourcePullPlan,
    approved: Option<&CatalogPullAction>,
    now: u64,
) -> CatalogActionResult {
    let item = &plan.item;
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
            message: Some(
                "Stored content was accepted; local content was left unchanged".to_string(),
            ),
        };
    };
    let resources = BTreeMap::from([(plan.resource.resource_id(), plan.resource.clone())]);
    match execute_one_action(
        environment,
        &plan.storage_id,
        &plan.plan_id,
        &resources,
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
    manifest_resources: &BTreeMap<CatalogResourceId, CatalogResource>,
    action: &CatalogPullAction,
    now: u64,
) -> Result<CatalogActionOutcome, String> {
    match &action.kind {
        CatalogPullActionKind::InstallPlugin { plan: native }
        | CatalogPullActionKind::UpdatePlugin { plan: native }
        | CatalogPullActionKind::OverwritePluginWithStorage { plan: native } => {
            let resource = manifest_resources
                .get(&action.resource_id)
                .ok_or("the plugin left the catalog after review")?;
            let CatalogResource::PluginInstallIntent(intent) = resource else {
                return Err("the plugin resource changed kind after review".to_string());
            };
            let LocalCatalogTargetId::Plugin { canonical_home, .. } = &action.target else {
                return Err("plugin action target mismatch".to_string());
            };
            let outcome = execute_plugin_action(
                environment.repository,
                environment.runner,
                storage_id,
                plan_id,
                action,
                native,
                &intent.descriptor,
                Path::new(canonical_home),
                now,
            )?;
            if !matches!(
                outcome,
                CatalogActionOutcome::Failed | CatalogActionOutcome::Partial
            ) {
                record_claim(
                    environment.repository,
                    storage_id,
                    &action.target,
                    &action.resource_id,
                    &action.incoming_content_sha256,
                    outcome,
                    plan_id,
                    now,
                )?;
            }
            Ok(outcome)
        }
        CatalogPullActionKind::InstallSkill
        | CatalogPullActionKind::UpdateSkill
        | CatalogPullActionKind::OverwriteSkillWithStorage
        | CatalogPullActionKind::InstallPet
        | CatalogPullActionKind::UpdatePet
        | CatalogPullActionKind::OverwritePetWithStorage => {
            let resource = manifest_resources
                .get(&action.resource_id)
                .ok_or("the resource left the catalog after review")?;
            let incoming = incoming_payload_for(resource, environment.fetch_payload)?;
            let target_path = action
                .target_path
                .as_ref()
                .ok_or("directory action lost its target path")?;
            replace_directory_target(
                environment.repository,
                DirectoryReplaceRequest {
                    storage_id: storage_id.clone(),
                    plan_id: plan_id.clone(),
                    action_id: action.action_id.clone(),
                    target: &action.target,
                    target_path: Path::new(target_path),
                    expected: &action.expected_target_state,
                    incoming: &incoming,
                    now,
                },
            )?;
            let outcome = match (&action.kind, &action.expected_target_state) {
                (_, CatalogTargetExpectation::Absent) => CatalogActionOutcome::Installed,
                (
                    CatalogPullActionKind::OverwriteSkillWithStorage
                    | CatalogPullActionKind::OverwritePetWithStorage,
                    _,
                ) => CatalogActionOutcome::Updated,
                _ => CatalogActionOutcome::Updated,
            };
            record_claim(
                environment.repository,
                storage_id,
                &action.target,
                &action.resource_id,
                &action.incoming_content_sha256,
                outcome,
                plan_id,
                now,
            )?;
            Ok(outcome)
        }
        CatalogPullActionKind::PreserveSkillAsCopyAndOverwrite { copy }
        | CatalogPullActionKind::PreservePetAsCopyAndOverwrite { copy } => {
            let resource = manifest_resources
                .get(&action.resource_id)
                .ok_or("the resource left the catalog after review")?;
            let incoming = incoming_payload_for(resource, environment.fetch_payload)?;
            let target_path = action
                .target_path
                .as_ref()
                .ok_or("preservation action lost its target path")?;
            preserve_and_overwrite(
                environment.repository,
                PreservationRequest {
                    storage_id: storage_id.clone(),
                    plan_id: plan_id.clone(),
                    action_id: action.action_id.clone(),
                    resource_id: action.resource_id.clone(),
                    target: &action.target,
                    target_path: Path::new(target_path),
                    pin: copy,
                    incoming: &incoming,
                    now,
                },
            )?;
            record_claim(
                environment.repository,
                storage_id,
                &action.target,
                &action.resource_id,
                &action.incoming_content_sha256,
                CatalogActionOutcome::PreservedAndUpdated,
                plan_id,
                now,
            )?;
            Ok(CatalogActionOutcome::PreservedAndUpdated)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn record_claim(
    repository: &CatalogStateRepository,
    storage_id: &StorageId,
    target: &LocalCatalogTargetId,
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

/// Startup recovery over the operations journal: verify, roll
/// back, or report every unfinished skill/pet operation, and never replay a
/// plugin operation automatically.
pub fn recover_catalog_operations(
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
            CatalogOperationKind::PluginApply => {
                // Never replay stale native arguments; fresh inventory decides.
                journal_phase(
                    repository,
                    &entry.operation_id,
                    CatalogOperationPhase::Failed,
                    Some(CatalogActionOutcome::Partial),
                    Some(
                        "a plugin operation was interrupted; Refresh to re-inventory the provider"
                            .to_string(),
                    ),
                    now,
                )?;
                messages.push(format!(
                    "A {} plugin operation was interrupted and needs a fresh review",
                    match entry.target {
                        LocalCatalogTargetId::Plugin { provider, .. } => provider.as_str(),
                        _ => "provider",
                    }
                ));
            }
            CatalogOperationKind::DirectoryReplace
            | CatalogOperationKind::Preservation
            | CatalogOperationKind::Fork => {
                let target_path = entry.target_path.as_ref().map(PathBuf::from);
                let copy_path = entry.copy_target_path.as_ref().map(PathBuf::from);
                let backup_path = entry.backup_path.as_ref().map(PathBuf::from);

                // A verified copy survives recovery untouched.
                let copy_ok = match (&copy_path, &entry.copy_content_sha256) {
                    (Some(copy), Some(expected)) => {
                        observed_directory_digest(
                            &preserved_target_for(
                                &entry.target,
                                entry.copy_name.as_deref().unwrap_or("copy"),
                            ),
                            copy,
                        )
                        .ok()
                        .flatten()
                        .as_deref()
                            == Some(expected.as_str())
                    }
                    _ => false,
                };

                // The original target: verified incoming content counts as
                // completed; otherwise restore the backup when it is safe.
                let mut restored = false;
                if let (Some(target_path), Some(backup)) = (&target_path, &backup_path) {
                    let target_state = observed_directory_digest(&entry.target, target_path)
                        .ok()
                        .flatten();
                    let completed = entry
                        .incoming_content_sha256
                        .as_ref()
                        .is_some_and(|expected| target_state.as_deref() == Some(expected.as_str()));
                    if !completed && target_state.is_none() && backup.exists() {
                        restored = fs::rename(backup, target_path).is_ok();
                    }
                }

                let phase = if restored {
                    CatalogOperationPhase::RolledBack
                } else {
                    CatalogOperationPhase::Failed
                };
                let message = if copy_ok {
                    "interrupted; the preserved copy is intact and will be reused on retry"
                } else if restored {
                    "interrupted; the original was restored from its backup"
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
                    entry
                        .copy_name
                        .as_deref()
                        .unwrap_or(entry.target_path.as_deref().unwrap_or("a target")),
                    message
                ));
                // Witnesses for missing or modified copies are invalid.
                if !copy_ok {
                    repository.mutate_preservations(|document| {
                        document
                            .witnesses
                            .retain(|witness| witness.operation_id != entry.operation_id);
                        Ok(())
                    })?;
                }
                // Stray stages are removed; backups are never deleted.
                if let Some(target_path) = &target_path {
                    if let Some(root) = target_path.parent() {
                        let _ = fs::remove_dir_all(stage_dir_for(root, &entry.operation_id, ""));
                        let _ =
                            fs::remove_dir_all(stage_dir_for(root, &entry.operation_id, "-copy"));
                        let _ = fs::remove_dir_all(stage_dir_for(
                            root,
                            &entry.operation_id,
                            "-incoming",
                        ));
                        let _ =
                            fs::remove_dir_all(stage_dir_for(root, &entry.operation_id, "-fork"));
                    }
                }
            }
        }
    }
    Ok(messages)
}
