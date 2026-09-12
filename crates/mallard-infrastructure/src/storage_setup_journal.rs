//! Bounded, non-secret recovery journal for storage setup probes.
//!
//! Each probe attempt is stored in its own schema-1 JSON document below the
//! machine-local Mallard state root. The shape is provider-neutral: the
//! storage provider key and config version, a stage, a sanitized error code,
//! timestamps, and a bounded, non-secret `facts` object the adapter types.
//! The schema deliberately has nowhere to put credentials, token values,
//! HTTP material, or local paths.

use crate::persistence::{read_json_bounded, write_json_atomic, LocalStateRepository};
use mallard_domain::{BoundedConfigObject, StorageProviderKey};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const STORAGE_SETUP_JOURNAL_SCHEMA: u32 = 1;
pub const MAX_STORAGE_SETUP_JOURNAL_ENTRIES: usize = 128;
pub const STORAGE_SETUP_ID_PREFIX: &str = "setup";

const MAX_STORAGE_SETUP_JOURNAL_BYTES: u64 = 96 * 1024;
const MAX_STORAGE_SETUP_DIRECTORY_ENTRIES: usize = MAX_STORAGE_SETUP_JOURNAL_ENTRIES * 2;
const STORAGE_SETUP_ID_HEX_BYTES: usize = 24;
const MAX_ERROR_CODE_BYTES: usize = 64;

static STORAGE_SETUP_JOURNAL_LOCK: Mutex<()> = Mutex::new(());

/// One strictly validated setup identifier (`setup-` plus 24 lowercase hex
/// characters). The fixed shape also makes it safe to use as a journal
/// filename and as a remote probe-key suffix.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StorageSetupId(String);

impl StorageSetupId {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_lower_hex_id(
            &value,
            &format!("{STORAGE_SETUP_ID_PREFIX}-"),
            STORAGE_SETUP_ID_HEX_BYTES,
            "storage setup ID",
        )?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StorageSetupId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for StorageSetupId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StorageSetupId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Durable capability-probe checkpoints. A completed probe has no journal
/// entry; it is removed only after the probe and its cleanup are confirmed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageSetupStage {
    Provisioning,
    ProbeRunning,
    CleanupRequired,
}

/// Schema-1 machine-local recovery state. All free-form remote or filesystem
/// data is intentionally excluded from this type; provider facts live only
/// inside the bounded object.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StorageSetupJournalEntry {
    pub schema: u32,
    pub setup_id: StorageSetupId,
    pub provider: StorageProviderKey,
    pub config_version: u32,
    pub stage: StorageSetupStage,
    pub error_code: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub facts: BoundedConfigObject,
    pub recovery: BoundedConfigObject,
}

impl StorageSetupJournalEntry {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != STORAGE_SETUP_JOURNAL_SCHEMA {
            return Err(format!(
                "unsupported storage setup journal schema {}",
                self.schema
            ));
        }
        // Re-validate even though deserialization also protects the newtype.
        StorageSetupId::parse(self.setup_id.as_str())?;
        if self.config_version == 0 {
            return Err("storage setup journal config version must be positive".to_string());
        }
        if self.created_at == 0 {
            return Err("storage setup journal created_at must be nonzero".to_string());
        }
        if self.updated_at < self.created_at {
            return Err("storage setup journal updated_at precedes created_at".to_string());
        }
        if let Some(error_code) = &self.error_code {
            validate_error_code(error_code)?;
        }
        match (self.stage, self.error_code.is_some()) {
            (StorageSetupStage::Provisioning | StorageSetupStage::ProbeRunning, false)
            | (StorageSetupStage::CleanupRequired, true) => {}
            (StorageSetupStage::Provisioning | StorageSetupStage::ProbeRunning, true) => {
                return Err("a running storage probe cannot have an error code".to_string())
            }
            (StorageSetupStage::CleanupRequired, false) => {
                return Err("storage probe cleanup requires a sanitized error code".to_string())
            }
        }
        Ok(())
    }

    /// Entries match when the same provider reported the same canonical
    /// facts, regardless of config version.
    pub fn matches(&self, provider: &StorageProviderKey, facts: &BoundedConfigObject) -> bool {
        &self.provider == provider && &self.facts == facts
    }
}

/// Atomic, no-follow access to storage setup journal entries under one local
/// state repository.
#[derive(Clone, Debug)]
pub struct StorageSetupJournalStore {
    root: PathBuf,
    directory: PathBuf,
}

impl StorageSetupJournalStore {
    pub(crate) fn r2_provisioning(repository: &LocalStateRepository) -> Self {
        Self {
            root: repository.root().to_path_buf(),
            directory: repository.root().join("r2_setup_journal"),
        }
    }
    pub(crate) fn dropbox_provisioning(repository: &LocalStateRepository) -> Self {
        Self {
            root: repository.root().to_path_buf(),
            directory: repository.root().join("dropbox_setup_journal"),
        }
    }
    pub fn new(repository: &LocalStateRepository) -> Self {
        Self {
            root: repository.root().to_path_buf(),
            directory: repository.root().join("storage_setup_journal"),
        }
    }

    pub fn load(
        &self,
        setup_id: &StorageSetupId,
    ) -> Result<Option<StorageSetupJournalEntry>, String> {
        let _guard = journal_guard(&self.root)?;
        self.load_unlocked(setup_id)
    }

    pub fn list(&self) -> Result<Vec<StorageSetupJournalEntry>, String> {
        let _guard = journal_guard(&self.root)?;
        self.list_unlocked()
    }
    /// Save the latest bounded state for a setup attempt. Atomic replacement
    /// is intentional: each document is the recovery snapshot for one stable
    /// setup ID.
    pub fn save(&self, entry: &StorageSetupJournalEntry) -> Result<(), String> {
        entry.validate()?;
        let _guard = journal_guard(&self.root)?;
        ensure_journal_directory(&self.root, &self.directory)?;
        let path = self.entry_path(&entry.setup_id);
        let is_new = match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
            {
                false
            }
            Ok(_) => {
                return Err(format!(
                    "storage setup journal '{}' is not a regular file",
                    entry.setup_id
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => {
                return Err(format!(
                    "inspect storage setup journal '{}': {}",
                    entry.setup_id, error
                ))
            }
        };
        if is_new && self.list_unlocked()?.len() >= MAX_STORAGE_SETUP_JOURNAL_ENTRIES {
            return Err(format!(
                "too many storage setup journal entries (limit {})",
                MAX_STORAGE_SETUP_JOURNAL_ENTRIES
            ));
        }
        write_json_atomic(&self.root, &path, entry, MAX_STORAGE_SETUP_JOURNAL_BYTES)
            .map_err(|error| sanitize_local_state_error(&self.root, error))
    }

    pub fn remove(&self, setup_id: &StorageSetupId) -> Result<bool, String> {
        let _guard = journal_guard(&self.root)?;
        let path = self.entry_path(setup_id);
        ensure_no_symlinks(&self.root, &path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "storage setup journal '{}' is not a regular file",
                    setup_id
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "inspect storage setup journal '{}': {}",
                    setup_id, error
                ))
            }
        }
        fs::remove_file(&path)
            .map(|()| true)
            .map_err(|error| format!("remove storage setup journal '{}': {}", setup_id, error))
    }

    /// Find all entries one provider journaled for the same canonical facts.
    /// Results are newest-first with setup ID as a stable tie-breaker.
    pub fn find_matching(
        &self,
        provider: &StorageProviderKey,
        facts: &BoundedConfigObject,
    ) -> Result<Vec<StorageSetupJournalEntry>, String> {
        let _guard = journal_guard(&self.root)?;
        Ok(self
            .list_unlocked()?
            .into_iter()
            .filter(|entry| entry.matches(provider, facts))
            .collect())
    }

    fn entry_path(&self, setup_id: &StorageSetupId) -> PathBuf {
        self.directory.join(format!("{}.json", setup_id.as_str()))
    }

    fn load_unlocked(
        &self,
        setup_id: &StorageSetupId,
    ) -> Result<Option<StorageSetupJournalEntry>, String> {
        let path = self.entry_path(setup_id);
        let entry = read_json_bounded::<StorageSetupJournalEntry>(
            &self.root,
            &path,
            MAX_STORAGE_SETUP_JOURNAL_BYTES,
        )
        .map_err(|error| sanitize_local_state_error(&self.root, error))?;
        if let Some(entry) = &entry {
            entry.validate()?;
            if &entry.setup_id != setup_id {
                return Err(format!(
                    "storage setup journal file '{}' contains setup '{}'",
                    setup_id, entry.setup_id
                ));
            }
        }
        Ok(entry)
    }

    fn list_unlocked(&self) -> Result<Vec<StorageSetupJournalEntry>, String> {
        match fs::symlink_metadata(&self.directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => {
                return Err("storage setup journal storage is not a real directory".to_string())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("inspect storage setup journal storage: {}", error)),
        }
        ensure_no_symlinks(&self.root, &self.directory)?;
        let entries = fs::read_dir(&self.directory)
            .map_err(|error| format!("list storage setup journal storage: {}", error))?;
        let mut directory_entries = 0_usize;
        let mut journals = Vec::new();
        for entry in entries {
            directory_entries = directory_entries.saturating_add(1);
            if directory_entries > MAX_STORAGE_SETUP_DIRECTORY_ENTRIES {
                return Err(format!(
                    "too many files in storage setup journal storage (limit {})",
                    MAX_STORAGE_SETUP_DIRECTORY_ENTRIES
                ));
            }
            let entry = entry.map_err(|error| format!("list storage setup journal: {}", error))?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            if journals.len() >= MAX_STORAGE_SETUP_JOURNAL_ENTRIES {
                return Err(format!(
                    "too many storage setup journal entries (limit {})",
                    MAX_STORAGE_SETUP_JOURNAL_ENTRIES
                ));
            }
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("inspect storage setup journal entry: {}", error))?;
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err("storage setup journal entry is not a regular file".to_string());
            }
            let file_name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| "storage setup journal filename is not valid UTF-8".to_string())?;
            let setup_id = StorageSetupId::parse(file_name)?;
            let document = self
                .load_unlocked(&setup_id)?
                .ok_or_else(|| format!("storage setup journal '{}' disappeared", setup_id))?;
            journals.push(document);
        }
        journals.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.setup_id.cmp(&right.setup_id))
        });
        Ok(journals)
    }
}

struct JournalGuard {
    _process: std::sync::MutexGuard<'static, ()>,
    _file: fs::File,
}

fn journal_guard(root: &Path) -> Result<JournalGuard, String> {
    let process = STORAGE_SETUP_JOURNAL_LOCK
        .lock()
        .map_err(|_| "storage setup journal lock is poisoned".to_string())?;
    let path = root.join("storage_setup_journal.lock");
    ensure_no_symlinks(root, &path)?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(&path)
        .map_err(|error| format!("open storage setup journal lock: {}", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect storage setup journal lock: {}", error))?;
    if !metadata.file_type().is_file() {
        return Err("storage setup journal lock is not a regular file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.nlink() != 1 {
            return Err("storage setup journal lock has unsafe hard links".to_string());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("secure storage setup journal lock: {}", error))?;
    }
    file.lock()
        .map_err(|error| format!("lock storage setup journal: {}", error))?;
    Ok(JournalGuard {
        _process: process,
        _file: file,
    })
}

fn ensure_journal_directory(root: &Path, directory: &Path) -> Result<(), String> {
    ensure_no_symlinks(root, directory)?;
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err("storage setup journal storage is not a real directory".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700);
                match builder.create(directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(format!("create storage setup journal storage: {}", error))
                    }
                }
            }
            #[cfg(not(unix))]
            match fs::create_dir(directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!("create storage setup journal storage: {}", error))
                }
            }
        }
        Err(error) => return Err(format!("inspect storage setup journal storage: {}", error)),
    }
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("inspect storage setup journal storage: {}", error))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("storage setup journal storage is not a real directory".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("secure storage setup journal storage: {}", error))?;
    }
    ensure_no_symlinks(root, directory)
}

fn ensure_no_symlinks(root: &Path, destination: &Path) -> Result<(), String> {
    let relative = destination
        .strip_prefix(root)
        .map_err(|_| "storage setup journal path escapes local state".to_string())?;
    let mut current = root.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            current.push(component.as_os_str());
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("storage setup journal path traverses a symlink".to_string())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("inspect storage setup journal path: {}", error)),
        }
    }
    Ok(())
}

fn validate_lower_hex_id(
    value: &str,
    prefix: &str,
    suffix_bytes: usize,
    label: &str,
) -> Result<(), String> {
    let Some(suffix) = value.strip_prefix(prefix) else {
        return Err(format!("{} has an invalid prefix", label));
    };
    validate_lower_hex_exact(suffix, suffix_bytes, label)
}

fn validate_lower_hex_exact(value: &str, expected_bytes: usize, label: &str) -> Result<(), String> {
    if value.len() != expected_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{} must be exactly {} lowercase hexadecimal characters",
            label, expected_bytes
        ));
    }
    Ok(())
}

fn validate_error_code(error_code: &str) -> Result<(), String> {
    if error_code.is_empty()
        || error_code.len() > MAX_ERROR_CODE_BYTES
        || !error_code.as_bytes()[0].is_ascii_lowercase()
        || !error_code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err("storage setup error code is not a sanitized lowercase code".to_string());
    }
    Ok(())
}

fn sanitize_local_state_error(root: &Path, error: String) -> String {
    // Persistence errors may contain the machine-local absolute state path.
    // Keep the useful failure reason without exposing that host-specific path.
    error.replace(root.to_string_lossy().as_ref(), "<local state>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SETUP_A: &str = "setup-0123456789abcdef01234567";
    const SETUP_B: &str = "setup-1123456789abcdef01234567";

    fn store(temp: &tempfile::TempDir) -> StorageSetupJournalStore {
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        StorageSetupJournalStore::new(&repository)
    }

    fn facts(bucket: &str) -> BoundedConfigObject {
        BoundedConfigObject::from_json(&json!({
            "account_id": "0123456789abcdef0123456789abcdef",
            "bucket": bucket,
            "jurisdiction": "eu",
        }))
        .unwrap()
    }

    fn provider() -> StorageProviderKey {
        StorageProviderKey::parse("r2").unwrap()
    }

    fn entry(setup_id: &str) -> StorageSetupJournalEntry {
        StorageSetupJournalEntry {
            schema: STORAGE_SETUP_JOURNAL_SCHEMA,
            setup_id: StorageSetupId::parse(setup_id).unwrap(),
            provider: provider(),
            config_version: 1,
            stage: StorageSetupStage::ProbeRunning,
            error_code: None,
            created_at: 10,
            updated_at: 11,
            facts: facts("mallard-backup"),
            recovery: BoundedConfigObject::empty(),
        }
    }

    #[test]
    fn round_trips_lists_and_finds_matching_facts() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let first = entry(SETUP_A);
        let mut second = entry(SETUP_B);
        second.updated_at = 12;
        second.facts = facts("other-bucket");

        store.save(&first).unwrap();
        store.save(&second).unwrap();

        assert_eq!(store.load(&first.setup_id).unwrap(), Some(first.clone()));
        assert_eq!(
            store
                .list()
                .unwrap()
                .into_iter()
                .map(|entry| entry.setup_id)
                .collect::<Vec<_>>(),
            vec![second.setup_id.clone(), first.setup_id.clone()]
        );
        assert_eq!(
            store
                .find_matching(&provider(), &facts("mallard-backup"))
                .unwrap(),
            vec![first.clone()]
        );
        assert!(store
            .find_matching(
                &StorageProviderKey::parse("local").unwrap(),
                &facts("mallard-backup")
            )
            .unwrap()
            .is_empty());
        assert!(store.remove(&first.setup_id).unwrap());
        assert!(!store.remove(&first.setup_id).unwrap());
    }

    #[test]
    fn validation_rejects_bad_ids_fields_and_collection_bounds() {
        for invalid in [
            "setup-0123456789abcdef0123456",
            "setup-0123456789abcdef012345678",
            "setup-0123456789abcdef0123456A",
            "r2setup-0123456789abcdef01234567",
            "setup-../../../../outside",
        ] {
            assert!(
                StorageSetupId::parse(invalid).is_err(),
                "accepted {invalid}"
            );
        }

        let mut document = entry(SETUP_A);
        document.config_version = 0;
        assert!(document.validate().is_err());
        document.config_version = 1;
        document.error_code = Some("probe_cleanup_failed".to_string());
        assert!(document.validate().is_err());
        document.error_code = Some("raw response: bearer secret".to_string());
        assert!(document.validate().is_err());

        document.error_code = Some("probe_cleanup_failed".to_string());
        document.stage = StorageSetupStage::CleanupRequired;
        document.validate().unwrap();
        document.error_code = None;
        assert!(document.validate().is_err());
    }

    #[test]
    fn corrupt_and_oversized_documents_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let setup_id = StorageSetupId::parse(SETUP_A).unwrap();
        ensure_journal_directory(&store.root, &store.directory).unwrap();
        let path = store.entry_path(&setup_id);

        fs::write(&path, b"{not json}\n").unwrap();
        assert!(store.load(&setup_id).is_err());
        assert!(store.list().is_err());

        fs::write(
            &path,
            vec![b'x'; MAX_STORAGE_SETUP_JOURNAL_BYTES as usize + 1],
        )
        .unwrap();
        assert!(store.load(&setup_id).is_err());
        assert!(store.list().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directory_and_entry_are_rejected() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, &store.directory).unwrap();
        assert!(store.save(&entry(SETUP_A)).is_err());
        assert!(store.list().is_err());
        fs::remove_file(&store.directory).unwrap();

        ensure_journal_directory(&store.root, &store.directory).unwrap();
        let outside_file = outside.join("entry.json");
        fs::write(&outside_file, serde_json::to_vec(&entry(SETUP_A)).unwrap()).unwrap();
        let setup_id = StorageSetupId::parse(SETUP_A).unwrap();
        symlink(&outside_file, store.entry_path(&setup_id)).unwrap();
        assert!(store.load(&setup_id).is_err());
        assert!(store.list().is_err());
        assert!(store.remove(&setup_id).is_err());
    }

    #[test]
    fn schema_is_provider_neutral_with_no_secret_or_path_channel() {
        let secret_canary = "mallard-secret-canary-value";
        let absolute_path_canary = "/Users/example/.mallard/private.json";
        let document = entry(SETUP_A);
        let serialized = serde_json::to_string(&document).unwrap();

        assert!(!serialized.contains(secret_canary));
        assert!(!serialized.contains(absolute_path_canary));
        for forbidden_key in [
            "account_id\":", // only inside facts, never top level
            "token_value",
            "secret_access_key",
            "authorization",
            "raw_body",
            "local_path",
        ] {
            let top_level: serde_json::Value = serde_json::from_str(&serialized).unwrap();
            assert!(top_level
                .get(forbidden_key.trim_end_matches("\":"))
                .is_none());
        }

        let mut value = serde_json::to_value(&document).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("token_value".to_string(), json!(secret_canary));
        assert!(serde_json::from_value::<StorageSetupJournalEntry>(value.clone()).is_err());
        value.as_object_mut().unwrap().remove("token_value");
        value
            .as_object_mut()
            .unwrap()
            .insert("bucket".to_string(), json!("top-level-r2-field"));
        assert!(serde_json::from_value::<StorageSetupJournalEntry>(value).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn saved_documents_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let document = entry(SETUP_A);
        store.save(&document).unwrap();

        let mode = fs::metadata(store.entry_path(&document.setup_id))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

/// The journal writer for one probe. Destination facts never change.
pub struct ProbeJournal<'a> {
    pub store: &'a StorageSetupJournalStore,
    pub entry: &'a mut StorageSetupJournalEntry,
}
impl crate::storage_providers::traits::StorageProbeJournal for ProbeJournal<'_> {
    fn recovery(&self) -> &BoundedConfigObject {
        &self.entry.recovery
    }
    fn checkpoint(&mut self, recovery: BoundedConfigObject) -> Result<(), String> {
        self.entry.recovery = recovery;
        self.entry.updated_at = self.entry.updated_at.saturating_add(1);
        self.store.save(self.entry)
    }
}
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct MemoryProbeJournal(pub BoundedConfigObject);
#[cfg(any(test, feature = "test-support"))]
impl crate::storage_providers::traits::StorageProbeJournal for MemoryProbeJournal {
    fn recovery(&self) -> &BoundedConfigObject {
        &self.0
    }
    fn checkpoint(&mut self, recovery: BoundedConfigObject) -> Result<(), String> {
        self.0 = recovery;
        Ok(())
    }
}
