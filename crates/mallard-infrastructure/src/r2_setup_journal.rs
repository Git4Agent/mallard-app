//! Bounded, non-secret recovery journal for Cloudflare R2 setup.
//!
//! Each setup attempt is stored in its own schema-1 JSON document below the
//! machine-local Mallard state root. The schema deliberately has nowhere to
//! put credentials, token values, HTTP material, or local paths.

use crate::persistence::{read_json_bounded, write_json_atomic, LocalStateRepository};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const R2_SETUP_JOURNAL_SCHEMA: u32 = 1;
pub const MAX_R2_SETUP_JOURNAL_ENTRIES: usize = 128;

const MAX_R2_SETUP_JOURNAL_BYTES: u64 = 16 * 1024;
const MAX_R2_SETUP_DIRECTORY_ENTRIES: usize = MAX_R2_SETUP_JOURNAL_ENTRIES * 2;
const R2_SETUP_ID_PREFIX: &str = "r2setup-";
const R2_SETUP_ID_HEX_BYTES: usize = 24;
const ACCOUNT_ID_HEX_BYTES: usize = 32;
const CREATED_TOKEN_ID_HEX_BYTES: usize = 32;
const MIN_BUCKET_BYTES: usize = 3;
const MAX_BUCKET_BYTES: usize = 63;
const MAX_ERROR_CODE_BYTES: usize = 64;

static R2_SETUP_JOURNAL_LOCK: Mutex<()> = Mutex::new(());

/// One strictly validated setup identifier. The fixed shape also makes it
/// safe to use as a journal filename and as the deterministic probe suffix.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct R2SetupId(String);

impl R2SetupId {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_lower_hex_id(
            &value,
            R2_SETUP_ID_PREFIX,
            R2_SETUP_ID_HEX_BYTES,
            "R2 setup ID",
        )?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for R2SetupId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for R2SetupId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for R2SetupId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Cloudflare R2's canonical endpoint jurisdiction.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum R2SetupJurisdiction {
    Default,
    Eu,
    Fedramp,
}

/// Durable provisioning checkpoints. A completed setup has no journal entry;
/// it is removed only after config commit and all cleanup are confirmed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum R2SetupStage {
    Started,
    TemporaryTokenCreated,
    BucketReady,
    FinalTokenCreated,
    ProbeRunning,
    ConfigCommitted,
    CleanupRequired,
}

/// Schema-1 machine-local recovery state. All free-form remote or filesystem
/// data is intentionally excluded from this type.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct R2SetupJournalEntry {
    pub schema: u32,
    pub setup_id: R2SetupId,
    pub account_id: String,
    pub bucket: String,
    pub jurisdiction: R2SetupJurisdiction,
    pub stage: R2SetupStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporary_token_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_token_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_token_id: Option<String>,
    pub bucket_created: bool,
    pub created_at: u64,
    pub updated_at: u64,
    pub error_code: Option<String>,
    pub probe_cleanup_needed: bool,
}

impl R2SetupJournalEntry {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != R2_SETUP_JOURNAL_SCHEMA {
            return Err(format!(
                "unsupported R2 setup journal schema {}",
                self.schema
            ));
        }
        // Re-validate even though deserialization also protects the newtype.
        R2SetupId::parse(self.setup_id.as_str())?;
        validate_lower_hex_exact(&self.account_id, ACCOUNT_ID_HEX_BYTES, "R2 account ID")?;
        validate_bucket(&self.bucket)?;
        let token_ids = [
            self.temporary_token_id.as_deref(),
            self.final_token_id.as_deref(),
            self.cleanup_token_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        for token_id in &token_ids {
            validate_lower_hex_exact(token_id, CREATED_TOKEN_ID_HEX_BYTES, "created R2 token ID")?;
        }
        for (index, token_id) in token_ids.iter().enumerate() {
            if token_ids[index + 1..].contains(token_id) {
                return Err("duplicate created R2 token ID roles".to_string());
            }
        }
        if self.created_at == 0 {
            return Err("R2 setup journal created_at must be nonzero".to_string());
        }
        if self.updated_at < self.created_at {
            return Err("R2 setup journal updated_at precedes created_at".to_string());
        }
        if let Some(error_code) = &self.error_code {
            validate_error_code(error_code)?;
        }
        if self.probe_cleanup_needed && self.stage != R2SetupStage::CleanupRequired {
            return Err(
                "R2 probe cleanup may be requested only at the cleanup-required stage".to_string(),
            );
        }
        Ok(())
    }

    pub fn matches_destination(
        &self,
        account_id: &str,
        jurisdiction: R2SetupJurisdiction,
        bucket: &str,
    ) -> bool {
        self.account_id == account_id && self.jurisdiction == jurisdiction && self.bucket == bucket
    }
}

/// Atomic, no-follow access to R2 setup journal entries under one local state
/// repository.
#[derive(Clone, Debug)]
pub struct R2SetupJournalStore {
    root: PathBuf,
    directory: PathBuf,
}

impl R2SetupJournalStore {
    pub fn new(repository: &LocalStateRepository) -> Self {
        Self {
            root: repository.root().to_path_buf(),
            directory: repository.root().join("r2_setup_transactions"),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn load(&self, setup_id: &R2SetupId) -> Result<Option<R2SetupJournalEntry>, String> {
        let _guard = journal_guard(&self.root)?;
        self.load_unlocked(setup_id)
    }

    pub fn list(&self) -> Result<Vec<R2SetupJournalEntry>, String> {
        let _guard = journal_guard(&self.root)?;
        self.list_unlocked()
    }

    /// Save the latest bounded state for a setup attempt. Atomic replacement
    /// is intentional: each document is the recovery snapshot for one stable
    /// setup ID.
    pub fn save(&self, entry: &R2SetupJournalEntry) -> Result<(), String> {
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
                    "R2 setup journal '{}' is not a regular file",
                    entry.setup_id
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => {
                return Err(format!(
                    "inspect R2 setup journal '{}': {}",
                    entry.setup_id, error
                ))
            }
        };
        if is_new && self.list_unlocked()?.len() >= MAX_R2_SETUP_JOURNAL_ENTRIES {
            return Err(format!(
                "too many R2 setup journal entries (limit {})",
                MAX_R2_SETUP_JOURNAL_ENTRIES
            ));
        }
        write_json_atomic(&self.root, &path, entry, MAX_R2_SETUP_JOURNAL_BYTES)
            .map_err(|error| sanitize_local_state_error(&self.root, error))
    }

    pub fn remove(&self, setup_id: &R2SetupId) -> Result<bool, String> {
        let _guard = journal_guard(&self.root)?;
        let path = self.entry_path(setup_id);
        ensure_no_symlinks(&self.root, &path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "R2 setup journal '{}' is not a regular file",
                    setup_id
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "inspect R2 setup journal '{}': {}",
                    setup_id, error
                ))
            }
        }
        fs::remove_file(&path)
            .map(|()| true)
            .map_err(|error| format!("remove R2 setup journal '{}': {}", setup_id, error))
    }

    /// Find all entries for one already-canonical destination. Results are
    /// newest-first with setup ID as a stable tie-breaker.
    pub fn find_matching(
        &self,
        account_id: &str,
        jurisdiction: R2SetupJurisdiction,
        bucket: &str,
    ) -> Result<Vec<R2SetupJournalEntry>, String> {
        validate_lower_hex_exact(account_id, ACCOUNT_ID_HEX_BYTES, "R2 account ID")?;
        validate_bucket(bucket)?;
        let _guard = journal_guard(&self.root)?;
        Ok(self
            .list_unlocked()?
            .into_iter()
            .filter(|entry| entry.matches_destination(account_id, jurisdiction, bucket))
            .collect())
    }

    fn entry_path(&self, setup_id: &R2SetupId) -> PathBuf {
        self.directory.join(format!("{}.json", setup_id.as_str()))
    }

    fn load_unlocked(&self, setup_id: &R2SetupId) -> Result<Option<R2SetupJournalEntry>, String> {
        let path = self.entry_path(setup_id);
        let entry =
            read_json_bounded::<R2SetupJournalEntry>(&self.root, &path, MAX_R2_SETUP_JOURNAL_BYTES)
                .map_err(|error| sanitize_local_state_error(&self.root, error))?;
        if let Some(entry) = &entry {
            entry.validate()?;
            if &entry.setup_id != setup_id {
                return Err(format!(
                    "R2 setup journal file '{}' contains setup '{}'",
                    setup_id, entry.setup_id
                ));
            }
        }
        Ok(entry)
    }

    fn list_unlocked(&self) -> Result<Vec<R2SetupJournalEntry>, String> {
        match fs::symlink_metadata(&self.directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err("R2 setup journal storage is not a real directory".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("inspect R2 setup journal storage: {}", error)),
        }
        ensure_no_symlinks(&self.root, &self.directory)?;
        let entries = fs::read_dir(&self.directory)
            .map_err(|error| format!("list R2 setup journal storage: {}", error))?;
        let mut directory_entries = 0_usize;
        let mut journals = Vec::new();
        for entry in entries {
            directory_entries = directory_entries.saturating_add(1);
            if directory_entries > MAX_R2_SETUP_DIRECTORY_ENTRIES {
                return Err(format!(
                    "too many files in R2 setup journal storage (limit {})",
                    MAX_R2_SETUP_DIRECTORY_ENTRIES
                ));
            }
            let entry = entry.map_err(|error| format!("list R2 setup journal: {}", error))?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            if journals.len() >= MAX_R2_SETUP_JOURNAL_ENTRIES {
                return Err(format!(
                    "too many R2 setup journal entries (limit {})",
                    MAX_R2_SETUP_JOURNAL_ENTRIES
                ));
            }
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("inspect R2 setup journal entry: {}", error))?;
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err("R2 setup journal entry is not a regular file".to_string());
            }
            let file_name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| "R2 setup journal filename is not valid UTF-8".to_string())?;
            let setup_id = R2SetupId::parse(file_name)?;
            let document = self
                .load_unlocked(&setup_id)?
                .ok_or_else(|| format!("R2 setup journal '{}' disappeared", setup_id))?;
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
    let process = R2_SETUP_JOURNAL_LOCK
        .lock()
        .map_err(|_| "R2 setup journal lock is poisoned".to_string())?;
    let path = root.join("r2_setup_transactions.lock");
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
        .map_err(|error| format!("open R2 setup journal lock: {}", error))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect R2 setup journal lock: {}", error))?;
    if !metadata.file_type().is_file() {
        return Err("R2 setup journal lock is not a regular file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.nlink() != 1 {
            return Err("R2 setup journal lock has unsafe hard links".to_string());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("secure R2 setup journal lock: {}", error))?;
    }
    file.lock()
        .map_err(|error| format!("lock R2 setup journal: {}", error))?;
    Ok(JournalGuard {
        _process: process,
        _file: file,
    })
}

fn ensure_journal_directory(root: &Path, directory: &Path) -> Result<(), String> {
    ensure_no_symlinks(root, directory)?;
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err("R2 setup journal storage is not a real directory".to_string()),
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
                        return Err(format!("create R2 setup journal storage: {}", error))
                    }
                }
            }
            #[cfg(not(unix))]
            match fs::create_dir(directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(format!("create R2 setup journal storage: {}", error)),
            }
        }
        Err(error) => return Err(format!("inspect R2 setup journal storage: {}", error)),
    }
    let metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("inspect R2 setup journal storage: {}", error))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("R2 setup journal storage is not a real directory".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("secure R2 setup journal storage: {}", error))?;
    }
    ensure_no_symlinks(root, directory)
}

fn ensure_no_symlinks(root: &Path, destination: &Path) -> Result<(), String> {
    let relative = destination
        .strip_prefix(root)
        .map_err(|_| "R2 setup journal path escapes local state".to_string())?;
    let mut current = root.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            current.push(component.as_os_str());
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("R2 setup journal path traverses a symlink".to_string())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("inspect R2 setup journal path: {}", error)),
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

fn validate_bucket(bucket: &str) -> Result<(), String> {
    if !(MIN_BUCKET_BYTES..=MAX_BUCKET_BYTES).contains(&bucket.len())
        || !bucket
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || !bucket
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !bucket
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err("R2 bucket name is not canonical".to_string());
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
        return Err("R2 setup error code is not a sanitized lowercase code".to_string());
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

    const SETUP_A: &str = "r2setup-0123456789abcdef01234567";
    const SETUP_B: &str = "r2setup-1123456789abcdef01234567";
    const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
    const TOKEN_A: &str = "1123456789abcdef0123456789abcdef";

    fn store(temp: &tempfile::TempDir) -> R2SetupJournalStore {
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        R2SetupJournalStore::new(&repository)
    }

    fn entry(setup_id: &str) -> R2SetupJournalEntry {
        R2SetupJournalEntry {
            schema: R2_SETUP_JOURNAL_SCHEMA,
            setup_id: R2SetupId::parse(setup_id).unwrap(),
            account_id: ACCOUNT.to_string(),
            bucket: "mallard-backup".to_string(),
            jurisdiction: R2SetupJurisdiction::Eu,
            stage: R2SetupStage::TemporaryTokenCreated,
            temporary_token_id: Some(TOKEN_A.to_string()),
            final_token_id: None,
            cleanup_token_id: None,
            bucket_created: false,
            created_at: 10,
            updated_at: 11,
            error_code: None,
            probe_cleanup_needed: false,
        }
    }

    #[test]
    fn round_trips_lists_and_finds_canonical_destinations() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let first = entry(SETUP_A);
        let mut second = entry(SETUP_B);
        second.updated_at = 12;
        second.jurisdiction = R2SetupJurisdiction::Default;

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
                .find_matching(ACCOUNT, R2SetupJurisdiction::Eu, "mallard-backup")
                .unwrap(),
            vec![first.clone()]
        );
        assert!(store
            .find_matching(
                &ACCOUNT.to_ascii_uppercase(),
                R2SetupJurisdiction::Eu,
                "mallard-backup"
            )
            .is_err());
        assert!(store.remove(&first.setup_id).unwrap());
        assert!(!store.remove(&first.setup_id).unwrap());
    }

    #[test]
    fn validation_rejects_bad_ids_fields_and_collection_bounds() {
        for invalid in [
            "r2setup-0123456789abcdef0123456",
            "r2setup-0123456789abcdef012345678",
            "r2setup-0123456789abcdef0123456A",
            "other-0123456789abcdef01234567",
            "r2setup-../../../../outside",
        ] {
            assert!(R2SetupId::parse(invalid).is_err(), "accepted {invalid}");
        }

        let mut document = entry(SETUP_A);
        document.account_id = ACCOUNT.to_ascii_uppercase();
        assert!(document.validate().is_err());
        document.account_id = ACCOUNT.to_string();
        document.bucket = "/private/tmp/secret".to_string();
        assert!(document.validate().is_err());
        document.bucket = "mallard-backup".to_string();
        document.final_token_id = Some(TOKEN_A.to_string());
        assert!(document.validate().is_err());
        document.final_token_id = None;
        document.error_code = Some("raw response: bearer secret".to_string());
        assert!(document.validate().is_err());
    }

    #[test]
    fn corrupt_and_oversized_documents_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let setup_id = R2SetupId::parse(SETUP_A).unwrap();
        ensure_journal_directory(&store.root, store.directory()).unwrap();
        let path = store.entry_path(&setup_id);

        fs::write(&path, b"{not json}\n").unwrap();
        assert!(store.load(&setup_id).is_err());
        assert!(store.list().is_err());

        fs::write(&path, vec![b'x'; MAX_R2_SETUP_JOURNAL_BYTES as usize + 1]).unwrap();
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
        symlink(&outside, store.directory()).unwrap();
        assert!(store.save(&entry(SETUP_A)).is_err());
        assert!(store.list().is_err());
        fs::remove_file(store.directory()).unwrap();

        ensure_journal_directory(&store.root, store.directory()).unwrap();
        let outside_file = outside.join("entry.json");
        fs::write(&outside_file, serde_json::to_vec(&entry(SETUP_A)).unwrap()).unwrap();
        let setup_id = R2SetupId::parse(SETUP_A).unwrap();
        symlink(&outside_file, store.entry_path(&setup_id)).unwrap();
        assert!(store.load(&setup_id).is_err());
        assert!(store.list().is_err());
        assert!(store.remove(&setup_id).is_err());
    }

    #[test]
    fn schema_has_no_secret_or_path_channel() {
        let secret_canary = "mallard-secret-canary-value";
        let absolute_path_canary = "/Users/example/.mallard/private.json";
        let document = entry(SETUP_A);
        let serialized = serde_json::to_string(&document).unwrap();

        assert!(!serialized.contains(secret_canary));
        assert!(!serialized.contains(absolute_path_canary));
        for forbidden_key in [
            "bootstrap_token",
            "token_value",
            "secret_access_key",
            "authorization",
            "raw_body",
            "local_path",
        ] {
            assert!(!serialized.contains(forbidden_key));
        }

        let mut value = serde_json::to_value(&document).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("token_value".to_string(), json!(secret_canary));
        value
            .as_object_mut()
            .unwrap()
            .insert("local_path".to_string(), json!(absolute_path_canary));
        assert!(serde_json::from_value::<R2SetupJournalEntry>(value).is_err());
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
