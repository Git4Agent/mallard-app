//! Machine-local catalog state under `~/.mallard`: bounded, atomic JSON
//! documents, create-only immutable plans, permission-restricted backups, and
//! canonical-target locks.
//!
//! The catalog owns its documents end to end; it never reads project-sync
//! documents and project sync never reads these. Corrupt catalog state is
//! quarantined (bytes preserved, file renamed off its scanned name) and the
//! operation fails closed rather than silently resetting bases or claims.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use super::domain::{
    sha256_hex, CatalogMaterializationsDocument, CatalogOperationsDocument, CatalogPlanId,
    CatalogPreservationsDocument, CatalogProfilesDocument, CatalogResourcePlanDocument,
    CatalogScopeLinksDocument, CatalogScopesDocument,
};

const MAX_CATALOG_PROFILES_BYTES: u64 = 1024 * 1024;
const MAX_CATALOG_SCOPES_BYTES: u64 = 1024 * 1024;
const MAX_CATALOG_PLAN_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CATALOG_MATERIALIZATIONS_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CATALOG_PRESERVATIONS_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CATALOG_OPERATIONS_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CATALOG_SCOPE_LINKS_BYTES: u64 = 16 * 1024 * 1024;

/// One process-wide guard over catalog document read-modify-write, paired
/// with an OS lock so a second Mallard process serializes too.
static CATALOG_STATE_LOCK: Mutex<()> = Mutex::new(());

pub struct CatalogStateGuard<'a> {
    _process: std::sync::MutexGuard<'a, ()>,
    _file: fs::File,
}

/// Exclusive guard over one canonical provider target path.
pub struct CatalogTargetLock {
    _file: fs::File,
}

pub struct CatalogStateRepository {
    root: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogStorageDataClearResult {
    pub links_reset: u64,
    pub plans_removed: u64,
    pub claims_removed: u64,
    pub preservation_witnesses_removed: u64,
    pub terminal_operations_removed: u64,
}

impl CatalogStateRepository {
    pub fn from_user_home() -> Result<Self, String> {
        let home = dirs::home_dir().ok_or("could not resolve the user home directory")?;
        Self::from_home_dir(home)
    }

    pub fn from_home_dir(home: impl Into<PathBuf>) -> Result<Self, String> {
        let home = home.into();
        if !home.is_absolute() {
            return Err("catalog state home must be absolute".to_string());
        }
        let root = home.join(".mallard");
        let repository = Self { root };
        repository.ensure_root()?;
        Ok(repository)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn ensure_root(&self) -> Result<(), String> {
        if let Some(parent) = self.root.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create '{}': {}", parent.display(), error))?;
        }
        match fs::symlink_metadata(&self.root) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "'{}' must be a real directory",
                    self.root.display()
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&self.root)
                    .or_else(|error| {
                        if error.kind() == std::io::ErrorKind::AlreadyExists {
                            Ok(())
                        } else {
                            Err(error)
                        }
                    })
                    .map_err(|error| format!("create '{}': {}", self.root.display(), error))?;
            }
            Err(error) => return Err(format!("inspect '{}': {}", self.root.display(), error)),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700));
        }
        Ok(())
    }

    fn guard(&self) -> Result<CatalogStateGuard<'static>, String> {
        let process = CATALOG_STATE_LOCK
            .lock()
            .map_err(|_| "catalog state lock is poisoned".to_string())?;
        let locks_dir = self.root.join("locks");
        ensure_restricted_directory(&locks_dir)?;
        let file = open_lock_file(&locks_dir.join("catalog-state.lock"))?;
        file.lock()
            .map_err(|error| format!("lock catalog state: {}", error))?;
        Ok(CatalogStateGuard {
            _process: process,
            _file: file,
        })
    }

    /// The state guard for callers outside the catalog's own transactions —
    /// today only the whole-`~/.mallard` reset, which must hold this while it
    /// deletes the root so no catalog mutation is mid-write underneath it.
    pub fn acquire_state_guard(&self) -> Result<CatalogStateGuard<'static>, String> {
        self.guard()
    }

    /// Exclusive mutation lock for one canonical provider path
    /// (`locks/provider-<canonical-path-sha256>.lock`).
    pub fn acquire_target_lock(&self, canonical_path: &Path) -> Result<CatalogTargetLock, String> {
        let locks_dir = self.root.join("locks");
        ensure_restricted_directory(&locks_dir)?;
        let digest = sha256_hex(canonical_path.to_string_lossy().as_bytes());
        let name = format!("provider-{}.lock", digest);
        let file = open_lock_file(&locks_dir.join(name))?;
        file.lock()
            .map_err(|error| format!("lock provider target: {}", error))?;
        Ok(CatalogTargetLock { _file: file })
    }

    fn profiles_path(&self) -> PathBuf {
        self.root.join("catalog_resource_profiles.json")
    }

    fn scope_links_path(&self) -> PathBuf {
        self.root.join("catalog_scope_links.json")
    }

    fn scopes_path(&self) -> PathBuf {
        self.root.join("catalog_scopes.json")
    }

    fn plan_path(&self, plan_id: &CatalogPlanId) -> PathBuf {
        self.root
            .join("catalog_resource_plans")
            .join(format!("{}.json", plan_id))
    }

    fn materializations_path(&self) -> PathBuf {
        self.root.join("catalog_resource_materializations.json")
    }

    fn preservations_path(&self) -> PathBuf {
        self.root.join("catalog_resource_preservations.json")
    }

    fn operations_path(&self) -> PathBuf {
        self.root.join("catalog_resource_operations.json")
    }

    /// Permission-restricted backup directory for one plan's directory
    /// operations: `backups/catalog_resources/<plan-id>/`.
    pub fn backup_dir(&self, plan_id: &CatalogPlanId) -> Result<PathBuf, String> {
        let directory = self
            .root
            .join("backups")
            .join("catalog_resources")
            .join(plan_id.as_str());
        ensure_restricted_directory(&self.root.join("backups"))?;
        ensure_restricted_directory(&self.root.join("backups").join("catalog_resources"))?;
        ensure_restricted_directory(&directory)?;
        Ok(directory)
    }

    fn load_document<T: DeserializeOwned + Default>(
        &self,
        path: &Path,
        max_bytes: u64,
        label: &str,
    ) -> Result<T, String> {
        match read_json_bounded::<T>(&self.root, path, max_bytes) {
            Ok(Some(document)) => Ok(document),
            Ok(None) => Ok(T::default()),
            Err(BoundedReadError::Io(message)) => Err(message),
            Err(BoundedReadError::Corrupt(message)) => {
                self.quarantine(path, label)?;
                Err(format!(
                    "{} was corrupt and has been quarantined: {}",
                    label, message
                ))
            }
        }
    }

    /// Preserve corrupt bytes off the scanned filename, then fail closed.
    fn quarantine(&self, path: &Path, label: &str) -> Result<(), String> {
        let quarantine_dir = self.root.join("catalog_quarantine");
        ensure_restricted_directory(&quarantine_dir)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document");
        let destination = quarantine_dir.join(format!(
            "{}-{}.quarantined",
            file_name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0)
        ));
        fs::hard_link(path, &destination)
            .or_else(|_| fs::copy(path, &destination).map(|_| ()))
            .map_err(|error| format!("quarantine {}: {}", label, error))?;
        fs::remove_file(path).map_err(|error| format!("retire corrupt {}: {}", label, error))?;
        Ok(())
    }

    pub fn load_profiles(&self) -> Result<CatalogProfilesDocument, String> {
        let _guard = self.guard()?;
        let document: CatalogProfilesDocument = self.load_document(
            &self.profiles_path(),
            MAX_CATALOG_PROFILES_BYTES,
            "catalog profiles",
        )?;
        document.validate()?;
        Ok(document)
    }

    pub fn mutate_profiles<T>(
        &self,
        mutate: impl FnOnce(&mut CatalogProfilesDocument) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = self.guard()?;
        let mut document: CatalogProfilesDocument = self.load_document(
            &self.profiles_path(),
            MAX_CATALOG_PROFILES_BYTES,
            "catalog profiles",
        )?;
        document.validate()?;
        let value = mutate(&mut document)?;
        document.revision = document.revision.saturating_add(1);
        document.validate()?;
        write_json_atomic(
            &self.root,
            &self.profiles_path(),
            &document,
            MAX_CATALOG_PROFILES_BYTES,
        )?;
        Ok(value)
    }

    pub fn load_scopes(&self) -> Result<CatalogScopesDocument, String> {
        let _guard = self.guard()?;
        let document: CatalogScopesDocument = self.load_document(
            &self.scopes_path(),
            MAX_CATALOG_SCOPES_BYTES,
            "catalog scopes",
        )?;
        document.validate()?;
        Ok(document)
    }

    pub fn mutate_scopes<T>(
        &self,
        mutate: impl FnOnce(&mut CatalogScopesDocument) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = self.guard()?;
        let mut document: CatalogScopesDocument = self.load_document(
            &self.scopes_path(),
            MAX_CATALOG_SCOPES_BYTES,
            "catalog scopes",
        )?;
        document.validate()?;
        let value = mutate(&mut document)?;
        document.revision = document.revision.saturating_add(1);
        document.validate()?;
        write_json_atomic(
            &self.root,
            &self.scopes_path(),
            &document,
            MAX_CATALOG_SCOPES_BYTES,
        )?;
        Ok(value)
    }

    pub fn load_scope_links(&self) -> Result<CatalogScopeLinksDocument, String> {
        let _guard = self.guard()?;
        let document: CatalogScopeLinksDocument = self.load_document(
            &self.scope_links_path(),
            MAX_CATALOG_SCOPE_LINKS_BYTES,
            "catalog scope links",
        )?;
        document.validate()?;
        Ok(document)
    }

    pub fn mutate_scope_links<T>(
        &self,
        mutate: impl FnOnce(&mut CatalogScopeLinksDocument) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = self.guard()?;
        let mut document: CatalogScopeLinksDocument = self.load_document(
            &self.scope_links_path(),
            MAX_CATALOG_SCOPE_LINKS_BYTES,
            "catalog scope links",
        )?;
        document.validate()?;
        let value = mutate(&mut document)?;
        document.revision = document.revision.saturating_add(1);
        document.validate()?;
        write_json_atomic(
            &self.root,
            &self.scope_links_path(),
            &document,
            MAX_CATALOG_SCOPE_LINKS_BYTES,
        )?;
        Ok(value)
    }

    /// Plans are immutable: reusing a plan ID is always an error, and edits
    /// arrive as new plans that supersede (discard) the old file.
    pub fn save_resource_plan(&self, plan: &CatalogResourcePlanDocument) -> Result<(), String> {
        plan.validate()?;
        let _guard = self.guard()?;
        let path = self.plan_path(plan.plan_id());
        if fs::symlink_metadata(&path).is_ok() {
            return Err(format!(
                "catalog resource plan '{}' already exists",
                plan.plan_id()
            ));
        }
        self.ensure_directory(&self.root.join("catalog_resource_plans"))?;
        write_json_atomic(&self.root, &path, plan, MAX_CATALOG_PLAN_BYTES)
    }

    pub fn load_resource_plan(
        &self,
        plan_id: &CatalogPlanId,
    ) -> Result<CatalogResourcePlanDocument, String> {
        let _guard = self.guard()?;
        let document = read_json_bounded::<CatalogResourcePlanDocument>(
            &self.root,
            &self.plan_path(plan_id),
            MAX_CATALOG_PLAN_BYTES,
        )
        .map_err(BoundedReadError::into_message)?
        .ok_or_else(|| format!("catalog resource plan '{}' does not exist", plan_id))?;
        document.validate()?;
        if document.plan_id() != plan_id {
            return Err(format!(
                "catalog resource plan file '{}' holds a different plan",
                plan_id
            ));
        }
        Ok(document)
    }

    pub fn list_resource_plans(&self) -> Result<Vec<CatalogResourcePlanDocument>, String> {
        let _guard = self.guard()?;
        self.list_resource_plans_unlocked()
    }

    fn list_resource_plans_unlocked(&self) -> Result<Vec<CatalogResourcePlanDocument>, String> {
        let mut plans = Vec::new();
        let directory = self.root.join("catalog_resource_plans");
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(plans),
            Err(error) => return Err(format!("read '{}': {}", directory.display(), error)),
        };
        for entry in entries {
            let entry = entry.map_err(|error| format!("read catalog plan entry: {}", error))?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !name.ends_with(".json") {
                continue;
            }
            let plan = read_json_bounded::<CatalogResourcePlanDocument>(
                &self.root,
                &entry.path(),
                MAX_CATALOG_PLAN_BYTES,
            )
            .map_err(BoundedReadError::into_message)?
            .ok_or_else(|| format!("catalog resource plan '{}' disappeared", name))?;
            plan.validate()?;
            plans.push(plan);
        }
        Ok(plans)
    }

    pub fn discard_plan(&self, plan_id: &CatalogPlanId) -> Result<bool, String> {
        let _guard = self.guard()?;
        let path = self.plan_path(plan_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("discard catalog plan '{}': {}", plan_id, error)),
        }
    }

    pub fn load_materializations(&self) -> Result<CatalogMaterializationsDocument, String> {
        let _guard = self.guard()?;
        let document: CatalogMaterializationsDocument = self.load_document(
            &self.materializations_path(),
            MAX_CATALOG_MATERIALIZATIONS_BYTES,
            "catalog materializations",
        )?;
        document.validate()?;
        Ok(document)
    }

    pub fn mutate_materializations<T>(
        &self,
        mutate: impl FnOnce(&mut CatalogMaterializationsDocument) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = self.guard()?;
        let mut document: CatalogMaterializationsDocument = self.load_document(
            &self.materializations_path(),
            MAX_CATALOG_MATERIALIZATIONS_BYTES,
            "catalog materializations",
        )?;
        document.validate()?;
        let value = mutate(&mut document)?;
        document.revision = document.revision.saturating_add(1);
        document.validate()?;
        write_json_atomic(
            &self.root,
            &self.materializations_path(),
            &document,
            MAX_CATALOG_MATERIALIZATIONS_BYTES,
        )?;
        Ok(value)
    }

    pub fn load_preservations(&self) -> Result<CatalogPreservationsDocument, String> {
        let _guard = self.guard()?;
        let document: CatalogPreservationsDocument = self.load_document(
            &self.preservations_path(),
            MAX_CATALOG_PRESERVATIONS_BYTES,
            "catalog preservations",
        )?;
        document.validate()?;
        Ok(document)
    }

    pub fn mutate_preservations<T>(
        &self,
        mutate: impl FnOnce(&mut CatalogPreservationsDocument) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = self.guard()?;
        let mut document: CatalogPreservationsDocument = self.load_document(
            &self.preservations_path(),
            MAX_CATALOG_PRESERVATIONS_BYTES,
            "catalog preservations",
        )?;
        document.validate()?;
        let value = mutate(&mut document)?;
        document.revision = document.revision.saturating_add(1);
        document.validate()?;
        write_json_atomic(
            &self.root,
            &self.preservations_path(),
            &document,
            MAX_CATALOG_PRESERVATIONS_BYTES,
        )?;
        Ok(value)
    }

    pub fn load_operations(&self) -> Result<CatalogOperationsDocument, String> {
        let _guard = self.guard()?;
        let document: CatalogOperationsDocument = self.load_document(
            &self.operations_path(),
            MAX_CATALOG_OPERATIONS_BYTES,
            "catalog operations",
        )?;
        document.validate()?;
        Ok(document)
    }

    pub fn mutate_operations<T>(
        &self,
        mutate: impl FnOnce(&mut CatalogOperationsDocument) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = self.guard()?;
        let mut document: CatalogOperationsDocument = self.load_document(
            &self.operations_path(),
            MAX_CATALOG_OPERATIONS_BYTES,
            "catalog operations",
        )?;
        document.validate()?;
        let value = mutate(&mut document)?;
        // Prune terminal journal entries beyond a bounded recent window so
        // the document cannot grow without limit.
        let terminal: Vec<usize> = document
            .operations
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.phase.is_terminal())
            .map(|(index, _)| index)
            .collect();
        if terminal.len() > 64 {
            let drop_count = terminal.len() - 64;
            let drop: std::collections::BTreeSet<usize> =
                terminal.into_iter().take(drop_count).collect();
            let mut index = 0;
            document.operations.retain(|_| {
                let keep = !drop.contains(&index);
                index += 1;
                keep
            });
        }
        document.revision = document.revision.saturating_add(1);
        document.validate()?;
        write_json_atomic(
            &self.root,
            &self.operations_path(),
            &document,
            MAX_CATALOG_OPERATIONS_BYTES,
        )?;
        Ok(value)
    }

    /// Report the first active Library reference that prevents deleting a
    /// configured storage. Callers must ask the user to unlink/discard it;
    /// storage deletion never silently rewrites Library state.
    pub fn storage_reference_reason(
        &self,
        storage_id: &mallard_domain::StorageId,
    ) -> Result<Option<String>, String> {
        if self
            .load_scope_links()?
            .links
            .iter()
            .any(|link| &link.storage_id == storage_id)
        {
            return Ok(Some("a Library link uses this storage".to_string()));
        }
        if self.list_resource_plans()?.iter().any(|plan| match plan {
            CatalogResourcePlanDocument::Push(plan) => &plan.storage_id == storage_id,
            CatalogResourcePlanDocument::Pull(plan) => &plan.storage_id == storage_id,
            CatalogResourcePlanDocument::Fork(_) => false,
        }) {
            return Ok(Some("a Library review plan uses this storage".to_string()));
        }
        if self
            .load_materializations()?
            .claims
            .iter()
            .any(|claim| &claim.storage_id == storage_id)
        {
            return Ok(Some(
                "a Library materialization claim uses this storage".to_string(),
            ));
        }
        if self
            .load_preservations()?
            .witnesses
            .iter()
            .any(|witness| &witness.storage_id == storage_id)
        {
            return Ok(Some(
                "a Library preservation record uses this storage".to_string(),
            ));
        }
        if self.load_operations()?.operations.iter().any(|operation| {
            operation.storage_id.as_ref() == Some(storage_id) && !operation.phase.is_terminal()
        }) {
            return Ok(Some(
                "an unfinished Library operation uses this storage".to_string(),
            ));
        }
        Ok(None)
    }

    /// Hold the catalog-wide lock while the caller clears the shared remote
    /// namespaces and project-sync metadata, then detach every Library record
    /// for this destination. Scope links remain configured so a later Push
    /// can repopulate the now-empty storage.
    pub fn clear_storage_data<T>(
        &self,
        storage_id: &mallard_domain::StorageId,
        clear_remote_and_project_state: impl FnOnce() -> Result<T, String>,
    ) -> Result<(T, CatalogStorageDataClearResult), String> {
        let _guard = self.guard()?;
        let mut scope_links: CatalogScopeLinksDocument = self.load_document(
            &self.scope_links_path(),
            MAX_CATALOG_SCOPE_LINKS_BYTES,
            "catalog scope links",
        )?;
        let mut materializations: CatalogMaterializationsDocument = self.load_document(
            &self.materializations_path(),
            MAX_CATALOG_MATERIALIZATIONS_BYTES,
            "catalog materializations",
        )?;
        let mut preservations: CatalogPreservationsDocument = self.load_document(
            &self.preservations_path(),
            MAX_CATALOG_PRESERVATIONS_BYTES,
            "catalog preservations",
        )?;
        let mut operations: CatalogOperationsDocument = self.load_document(
            &self.operations_path(),
            MAX_CATALOG_OPERATIONS_BYTES,
            "catalog operations",
        )?;
        scope_links.validate()?;
        materializations.validate()?;
        preservations.validate()?;
        operations.validate()?;
        if operations.operations.iter().any(|operation| {
            operation.storage_id.as_ref() == Some(storage_id) && !operation.phase.is_terminal()
        }) {
            return Err(format!(
                "storage '{}' has an unfinished Library operation; finish or recover it before clearing the storage",
                storage_id
            ));
        }

        let plans = self.list_resource_plans_unlocked()?;
        let plan_ids = plans
            .into_iter()
            .filter(|plan| match plan {
                CatalogResourcePlanDocument::Push(plan) => &plan.storage_id == storage_id,
                CatalogResourcePlanDocument::Pull(plan) => &plan.storage_id == storage_id,
                CatalogResourcePlanDocument::Fork(_) => false,
            })
            .map(|plan| plan.plan_id().clone())
            .collect::<Vec<_>>();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock is before Unix epoch".to_string())?
            .as_secs();
        let mut links_reset = 0_u64;
        for link in scope_links
            .links
            .iter_mut()
            .filter(|link| &link.storage_id == storage_id)
        {
            if link.accepted_head.is_some() || !link.accepted_resources.is_empty() {
                link.accepted_head = None;
                link.accepted_resources.clear();
                link.revision = link.revision.saturating_add(1);
                link.updated_at = link.updated_at.max(now);
                links_reset = links_reset.saturating_add(1);
            }
        }
        let claim_count = materializations.claims.len();
        materializations
            .claims
            .retain(|claim| &claim.storage_id != storage_id);
        let claims_removed = (claim_count - materializations.claims.len()) as u64;
        let witness_count = preservations.witnesses.len();
        preservations
            .witnesses
            .retain(|witness| &witness.storage_id != storage_id);
        let preservation_witnesses_removed = (witness_count - preservations.witnesses.len()) as u64;
        let operation_count = operations.operations.len();
        operations
            .operations
            .retain(|operation| operation.storage_id.as_ref() != Some(storage_id));
        let terminal_operations_removed = (operation_count - operations.operations.len()) as u64;

        if links_reset > 0 {
            scope_links.revision = scope_links.revision.saturating_add(1);
        }
        if claims_removed > 0 {
            materializations.revision = materializations.revision.saturating_add(1);
        }
        if preservation_witnesses_removed > 0 {
            preservations.revision = preservations.revision.saturating_add(1);
        }
        if terminal_operations_removed > 0 {
            operations.revision = operations.revision.saturating_add(1);
        }
        scope_links.validate()?;
        materializations.validate()?;
        preservations.validate()?;
        operations.validate()?;

        let value = clear_remote_and_project_state()?;

        if links_reset > 0 {
            write_json_atomic(
                &self.root,
                &self.scope_links_path(),
                &scope_links,
                MAX_CATALOG_SCOPE_LINKS_BYTES,
            )?;
        }
        if claims_removed > 0 {
            write_json_atomic(
                &self.root,
                &self.materializations_path(),
                &materializations,
                MAX_CATALOG_MATERIALIZATIONS_BYTES,
            )?;
        }
        if preservation_witnesses_removed > 0 {
            write_json_atomic(
                &self.root,
                &self.preservations_path(),
                &preservations,
                MAX_CATALOG_PRESERVATIONS_BYTES,
            )?;
        }
        if terminal_operations_removed > 0 {
            write_json_atomic(
                &self.root,
                &self.operations_path(),
                &operations,
                MAX_CATALOG_OPERATIONS_BYTES,
            )?;
        }
        for plan_id in &plan_ids {
            let path = self.plan_path(plan_id);
            ensure_no_symlinks(&self.root, &path)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("discard catalog plan '{}': {}", plan_id, error)),
            }
        }

        Ok((
            value,
            CatalogStorageDataClearResult {
                links_reset,
                plans_removed: plan_ids.len() as u64,
                claims_removed,
                preservation_witnesses_removed,
                terminal_operations_removed,
            },
        ))
    }

    fn ensure_directory(&self, directory: &Path) -> Result<(), String> {
        if !directory.starts_with(&self.root) {
            return Err(format!(
                "'{}' escapes the catalog state root",
                directory.display()
            ));
        }
        match fs::symlink_metadata(directory) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(()),
            Ok(_) => Err(format!(
                "'{}' must be a real directory",
                directory.display()
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(directory)
                    .map_err(|error| format!("create '{}': {}", directory.display(), error))
            }
            Err(error) => Err(format!("inspect '{}': {}", directory.display(), error)),
        }
    }
}

fn ensure_restricted_directory(directory: &Path) -> Result<(), String> {
    match fs::symlink_metadata(directory) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(format!(
                "'{}' must be a real directory",
                directory.display()
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(directory)
                .map_err(|error| format!("create '{}': {}", directory.display(), error))?;
        }
        Err(error) => return Err(format!("inspect '{}': {}", directory.display(), error)),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(directory, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn open_lock_file(path: &Path) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|error| format!("open catalog lock '{}': {}", path.display(), error))
}

enum BoundedReadError {
    Corrupt(String),
    Io(String),
}

impl BoundedReadError {
    fn into_message(self) -> String {
        match self {
            BoundedReadError::Corrupt(message) | BoundedReadError::Io(message) => message,
        }
    }
}

fn ensure_no_symlinks(root: &Path, destination: &Path) -> Result<(), String> {
    if !destination.starts_with(root) {
        return Err(format!(
            "'{}' escapes the catalog state root",
            destination.display()
        ));
    }
    let mut cursor = root.to_path_buf();
    let relative = destination
        .strip_prefix(root)
        .map_err(|_| format!("'{}' escapes the catalog state root", destination.display()))?;
    for component in relative.components() {
        cursor.push(component);
        if let Ok(meta) = fs::symlink_metadata(&cursor) {
            if meta.file_type().is_symlink() {
                return Err(format!("symlink traversal at '{}'", cursor.display()));
            }
        }
    }
    Ok(())
}

fn read_json_bounded<T: DeserializeOwned>(
    root: &Path,
    path: &Path,
    max_bytes: u64,
) -> Result<Option<T>, BoundedReadError> {
    ensure_no_symlinks(root, path).map_err(BoundedReadError::Io)?;
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(BoundedReadError::Io(format!(
                "inspect '{}': {}",
                path.display(),
                error
            )))
        }
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(BoundedReadError::Io(format!(
            "'{}' is not a regular file",
            path.display()
        )));
    }
    if meta.len() > max_bytes {
        return Err(BoundedReadError::Corrupt(format!(
            "'{}' exceeds its read limit",
            path.display()
        )));
    }
    let file = fs::File::open(path)
        .map_err(|error| BoundedReadError::Io(format!("open '{}': {}", path.display(), error)))?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| BoundedReadError::Io(format!("read '{}': {}", path.display(), error)))?;
    if bytes.len() as u64 > max_bytes {
        return Err(BoundedReadError::Io(format!(
            "'{}' exceeds its read limit",
            path.display()
        )));
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        BoundedReadError::Corrupt(format!("parse '{}': {}", path.display(), error))
    })
}

fn write_json_atomic<T: Serialize>(
    root: &Path,
    path: &Path,
    value: &T,
    max_bytes: u64,
) -> Result<(), String> {
    ensure_no_symlinks(root, path)?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("'{}' has no parent", path.display()))?;
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("serialize '{}': {}", path.display(), error))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > max_bytes {
        return Err(format!("'{}' exceeds its write limit", path.display()));
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("create temp in '{}': {}", parent.display(), error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = temp
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600));
    }
    temp.as_file_mut()
        .write_all(&bytes)
        .map_err(|error| format!("write '{}': {}", path.display(), error))?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|error| format!("sync '{}': {}", path.display(), error))?;
    temp.persist(path)
        .map_err(|error| format!("publish '{}': {}", path.display(), error.error))?;
    sync_directory(parent);
    Ok(())
}

#[cfg(unix)]
fn sync_directory(directory: &Path) {
    if let Ok(handle) = fs::File::open(directory) {
        let _ = handle.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) {}

#[cfg(test)]
mod tests {
    use super::super::domain::{
        CatalogCommitId, CatalogLinkId, CatalogPlanId, CatalogProfilesDocument,
        CatalogResourceKindTag, CatalogResourceRepositoryId, CatalogScope, CatalogScopeId,
        CatalogScopeLink, CatalogScopesDocument, LibraryHeadCoordinate, LocalCatalogProfile,
        LocalCatalogProfileId, ProviderKey,
    };
    use super::*;
    use mallard_domain::StorageId;
    use std::collections::BTreeMap;

    fn repository(temp: &tempfile::TempDir) -> CatalogStateRepository {
        CatalogStateRepository::from_home_dir(temp.path()).unwrap()
    }

    #[test]
    fn documents_default_and_round_trip_with_revision_bumps() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        assert_eq!(
            repo.load_profiles().unwrap(),
            CatalogProfilesDocument::default()
        );
        repo.mutate_profiles(|document| {
            document.profiles.push(LocalCatalogProfile {
                profile_id: LocalCatalogProfileId::generate().unwrap(),
                provider_key: ProviderKey::Codex,
                display_name: "Codex".to_string(),
                configured_home: temp.path().join("codex").to_string_lossy().to_string(),
                revision: 1,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        let loaded = repo.load_profiles().unwrap();
        assert_eq!(loaded.revision, 1);
        assert_eq!(loaded.profiles.len(), 1);
    }

    #[test]
    fn scopes_document_round_trips_with_revision_bumps() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        assert_eq!(
            repo.load_scopes().unwrap(),
            CatalogScopesDocument::default()
        );
        repo.mutate_scopes(|document| {
            document.scopes.push(CatalogScope {
                scope_id: CatalogScopeId::generate().unwrap(),
                provider: ProviderKey::Codex,
                profile_id: LocalCatalogProfileId::generate().unwrap(),
                resource_kind: CatalogResourceKindTag::AgentSkill,
                revision: 1,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        let loaded = repo.load_scopes().unwrap();
        assert_eq!(loaded.revision, 1);
        assert_eq!(loaded.scopes.len(), 1);
        assert!(repo.root().join("catalog_scopes.json").exists());
    }

    #[test]
    fn corrupt_documents_quarantine_and_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        fs::write(
            repo.root().join("catalog_resource_profiles.json"),
            b"{not json",
        )
        .unwrap();
        let error = repo.load_profiles().unwrap_err();
        assert!(error.contains("quarantined"), "{error}");
        assert!(!repo.root().join("catalog_resource_profiles.json").exists());
        let quarantined: Vec<_> = fs::read_dir(repo.root().join("catalog_quarantine"))
            .unwrap()
            .collect();
        assert_eq!(quarantined.len(), 1);
        // After quarantine the store recovers to its default.
        assert_eq!(
            repo.load_profiles().unwrap(),
            CatalogProfilesDocument::default()
        );
    }

    #[test]
    fn plans_are_create_only() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let missing = CatalogPlanId::generate().unwrap();
        assert!(repo
            .load_resource_plan(&missing)
            .unwrap_err()
            .contains("does not exist"));
        assert!(!repo.discard_plan(&missing).unwrap());
    }

    #[test]
    fn legacy_catalog_documents_are_never_read() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        fs::write(repo.root().join("catalog_profiles.json"), b"{legacy").unwrap();
        fs::write(repo.root().join("storage_catalog.json"), b"{legacy").unwrap();
        assert_eq!(
            repo.load_profiles().unwrap(),
            CatalogProfilesDocument::default()
        );
        assert!(repo.load_scope_links().unwrap().links.is_empty());
        assert!(repo.root().join("catalog_profiles.json").exists());
        assert!(repo.root().join("storage_catalog.json").exists());
    }

    #[test]
    fn linked_storage_cannot_be_removed() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("storage-a").unwrap();
        repo.mutate_scope_links(|document| {
            document.links.push(CatalogScopeLink {
                link_id: CatalogLinkId::generate().unwrap(),
                scope_id: CatalogScopeId::generate().unwrap(),
                storage_id: storage_id.clone(),
                repository_id: CatalogResourceRepositoryId::for_kind(
                    ProviderKey::Codex,
                    CatalogResourceKindTag::AgentSkill,
                ),
                accepted_head: None,
                accepted_resources: BTreeMap::new(),
                revision: 1,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        assert!(repo
            .storage_reference_reason(&storage_id)
            .unwrap()
            .unwrap()
            .contains("link"));
    }

    #[test]
    fn storage_clear_keeps_catalog_links_and_forgets_their_accepted_heads() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("storage-a").unwrap();
        let repository_id = CatalogResourceRepositoryId::for_kind(
            ProviderKey::Codex,
            CatalogResourceKindTag::AgentSkill,
        );
        repo.mutate_scope_links(|document| {
            document.links.push(CatalogScopeLink {
                link_id: CatalogLinkId::generate().unwrap(),
                scope_id: CatalogScopeId::generate().unwrap(),
                storage_id: storage_id.clone(),
                repository_id: repository_id.clone(),
                accepted_head: Some(LibraryHeadCoordinate {
                    repository_id,
                    revision: 1,
                    commit_id: CatalogCommitId::generate().unwrap(),
                    manifest_sha256: "a".repeat(64),
                    manifest_size: 1,
                    storage_identity_sha256: "b".repeat(64),
                }),
                accepted_resources: BTreeMap::from([(
                    crate::storage_catalog::domain::skill_resource_id(
                        crate::storage_catalog::domain::AgentSkillScope::CodexHome,
                        "review",
                        "review",
                    ),
                    "c".repeat(64),
                )]),
                revision: 1,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        let mut callback_called = false;

        let (value, result) = repo
            .clear_storage_data(&storage_id, || {
                callback_called = true;
                Ok("cleared")
            })
            .unwrap();

        assert!(callback_called);
        assert_eq!(value, "cleared");
        assert_eq!(result.links_reset, 1);
        let links = repo.load_scope_links().unwrap();
        assert_eq!(links.links.len(), 1);
        assert!(links.links[0].accepted_head.is_none());
        assert!(links.links[0].accepted_resources.is_empty());
    }
}
