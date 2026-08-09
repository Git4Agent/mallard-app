//! Local-folder object store: no-follow traversal, atomic writes, and a
//! storage-wide advisory lock for the two mutating operations.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tempfile::NamedTempFile;
use walkdir::WalkDir;

use super::object_store::{
    CasExpectation, CasOutcome, ExpectedObject, ImmutablePutOutcome, ObjectStore, StageOutcome,
    StoreListPage, StoredMetadata, VerifiedBody,
};
use super::{
    hex_lower, sha256, sha256_reader, ObjectKey, ObjectPrefix, LOCAL_STORAGE_LOCK, MALLARD_ROOT,
    MAX_LIST_PAGE,
};
use mallard_domain as domain;

#[derive(Clone, Debug)]
pub struct LocalObjectStore {
    root: PathBuf,
}

impl LocalObjectStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref();
        if !root.is_absolute() {
            return Err(format!(
                "local bundle store must be absolute: '{}'",
                root.display()
            ));
        }
        if let Ok(meta) = fs::symlink_metadata(root) {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(format!(
                    "local store '{}' must be a real directory",
                    root.display()
                ));
            }
        } else {
            fs::create_dir_all(root)
                .map_err(|e| format!("create local store '{}': {}", root.display(), e))?;
        }
        let root = fs::canonicalize(root)
            .map_err(|e| format!("resolve local store '{}': {}", root.display(), e))?;
        let mallard_root = root.join(MALLARD_ROOT);
        match fs::symlink_metadata(&mallard_root) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "Mallard storage namespace '{}' must be a real directory",
                    mallard_root.display()
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&mallard_root) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(format!(
                            "create Mallard storage namespace '{}': {}",
                            mallard_root.display(),
                            error
                        ))
                    }
                }
                let meta = fs::symlink_metadata(&mallard_root).map_err(|error| {
                    format!(
                        "inspect Mallard storage namespace '{}': {}",
                        mallard_root.display(),
                        error
                    )
                })?;
                if meta.file_type().is_symlink() || !meta.is_dir() {
                    return Err(format!(
                        "Mallard storage namespace '{}' must be a real directory",
                        mallard_root.display()
                    ));
                }
            }
            Err(error) => {
                return Err(format!(
                    "inspect Mallard storage namespace '{}': {}",
                    mallard_root.display(),
                    error
                ))
            }
        }
        Ok(Self { root })
    }

    fn lock(&self) -> Result<fs::File, String> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(MALLARD_ROOT).join(LOCAL_STORAGE_LOCK))
            .map_err(|e| format!("open Mallard storage lock: {}", e))?;
        file.lock()
            .map_err(|e| format!("lock local Mallard storage: {}", e))?;
        Ok(file)
    }

    fn open_object_unlocked(
        &self,
        key: &ObjectKey,
    ) -> Result<Option<(fs::File, fs::Metadata)>, String> {
        let path = checked_existing_object_path(&self.root, key)?;
        let Some(path) = path else {
            return Ok(None);
        };
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("inspect '{}': {}", key, e))?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(format!("object '{}' is not a regular no-follow file", key));
        }
        let file = open_read_only_no_follow(&path, &format!("object '{}'", key))?;
        let opened_meta = file
            .metadata()
            .map_err(|error| format!("inspect opened object '{}': {}", key, error))?;
        if !opened_meta.is_file() || opened_meta.len() != meta.len() {
            return Err(format!("object '{}' changed while opening", key));
        }
        Ok(Some((file, opened_meta)))
    }

    fn read_metadata_unlocked(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        let Some((mut file, metadata)) = self.open_object_unlocked(key)? else {
            return Ok(None);
        };
        if metadata.len() > max_bytes {
            return Err(format!("metadata object '{}' exceeds its read limit", key));
        }
        let capacity = usize::try_from(metadata.len())
            .map_err(|_| format!("metadata object '{}' is too large for this platform", key))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| format!("reserve memory for metadata object '{}'", key))?;
        Read::by_ref(&mut file)
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read metadata object '{}': {}", key, error))?;
        if bytes.len() as u64 > max_bytes {
            return Err(format!("metadata object '{}' exceeds its read limit", key));
        }
        Ok(Some(StoredMetadata {
            etag: sha256(&bytes),
            bytes,
        }))
    }

    fn stage_verified_unlocked(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        domain::validate_sha256("expected object", &expected.sha256)?;
        let Some((mut source, metadata)) = self.open_object_unlocked(key)? else {
            return Ok(StageOutcome::Missing);
        };
        if metadata.len() != expected.size {
            return Ok(StageOutcome::Mismatch);
        }
        let mut staged = NamedTempFile::new()
            .map_err(|error| format!("create staged object '{}': {}", key, error))?;
        let mut hasher = Sha256::new();
        let mut received = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = source
                .read(&mut buffer)
                .map_err(|error| format!("read object '{}': {}", key, error))?;
            if count == 0 {
                break;
            }
            received = received
                .checked_add(count as u64)
                .ok_or_else(|| format!("object '{}' byte count overflow", key))?;
            if received > expected.size {
                return Ok(StageOutcome::Mismatch);
            }
            hasher.update(&buffer[..count]);
            staged
                .as_file_mut()
                .write_all(&buffer[..count])
                .map_err(|error| format!("stage object '{}': {}", key, error))?;
        }
        let digest = hex_lower(&hasher.finalize());
        if received != expected.size || digest != expected.sha256 {
            return Ok(StageOutcome::Mismatch);
        }
        staged
            .as_file_mut()
            .sync_all()
            .map_err(|error| format!("sync staged object '{}': {}", key, error))?;
        Ok(StageOutcome::Verified(VerifiedBody::from_temp_file(
            staged,
            received,
            digest.clone(),
            digest,
        )?))
    }

    fn current_etag_unlocked(&self, key: &ObjectKey) -> Result<Option<String>, String> {
        let Some((mut file, _)) = self.open_object_unlocked(key)? else {
            return Ok(None);
        };
        Ok(Some(sha256_reader(
            &mut file,
            &format!("object '{}'", key),
        )?))
    }

    fn write_atomic_unlocked(&self, key: &ObjectKey, bytes: &[u8]) -> Result<(), String> {
        let path = checked_create_object_path(&self.root, key)?;
        let parent = path
            .parent()
            .ok_or_else(|| format!("object '{}' has no parent", key))?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("create object temp in '{}': {}", parent.display(), e))?;
        temp.as_file_mut()
            .write_all(bytes)
            .map_err(|e| format!("write object '{}': {}", key, e))?;
        temp.as_file_mut()
            .sync_all()
            .map_err(|e| format!("sync object '{}': {}", key, e))?;
        temp.persist(&path)
            .map_err(|e| format!("publish object '{}': {}", key, e.error))?;
        Ok(())
    }
}

impl ObjectStore for LocalObjectStore {
    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        self.read_metadata_unlocked(key, max_bytes)
    }

    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        self.stage_verified_unlocked(key, expected)
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        let _guard = self.lock()?;
        if let Some((mut existing, metadata)) = self.open_object_unlocked(key)? {
            if metadata.len() == bytes.len() as u64
                && sha256_reader(&mut existing, &format!("object '{}'", key))? == sha256(bytes)
            {
                return Ok(ImmutablePutOutcome::AlreadyPresent);
            }
            return Err(format!(
                "immutable object '{}' already exists with different bytes",
                key
            ));
        }
        self.write_atomic_unlocked(key, bytes)?;
        Ok(ImmutablePutOutcome::Written)
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        let _guard = self.lock()?;
        let current_etag = self.current_etag_unlocked(key)?;
        let matches = match expectation {
            CasExpectation::Absent => current_etag.is_none(),
            CasExpectation::Match(expected) => current_etag.as_ref() == Some(expected),
        };
        if !matches {
            return Ok(CasOutcome::Conflict { current_etag });
        }
        self.write_atomic_unlocked(key, bytes)?;
        Ok(CasOutcome::Written {
            etag: sha256(bytes),
        })
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        if limit == 0 || limit > MAX_LIST_PAGE {
            return Err(format!(
                "list limit must be between 1 and {}",
                MAX_LIST_PAGE
            ));
        }
        if let Some(cursor) = cursor {
            ObjectKey::parse(cursor.to_string())?;
        }
        let namespace_root = self.root.join(prefix.as_str());
        let namespace_meta = match fs::symlink_metadata(&namespace_root) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoreListPage {
                    keys: Vec::new(),
                    next_cursor: None,
                })
            }
            Err(error) => {
                return Err(format!(
                    "inspect local Mallard repository namespace '{}': {}",
                    namespace_root.display(),
                    error
                ))
            }
        };
        if namespace_meta.file_type().is_symlink() || !namespace_meta.is_dir() {
            return Err(format!(
                "local Mallard repository namespace '{}' must be a real directory",
                namespace_root.display()
            ));
        }

        let mut keys = Vec::new();
        for entry in WalkDir::new(&namespace_root)
            .follow_links(false)
            .into_iter()
        {
            let entry = entry.map_err(|e| format!("walk local bundle store: {}", e))?;
            if entry.depth() == 0 {
                continue;
            }
            if entry.file_type().is_symlink() {
                return Err(format!(
                    "symlink '{}' is forbidden in local bundle storage",
                    entry.path().display()
                ));
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = normalized_relative_path(&self.root, entry.path())?;
            if relative.contains("/.tmp") {
                continue;
            }
            if relative == prefix.as_str() || relative.starts_with(&format!("{}/", prefix.as_str()))
            {
                let key = ObjectKey::parse(relative)?;
                if cursor.is_none_or(|cursor| key.as_str() > cursor) {
                    keys.push(key);
                }
            }
        }
        keys.sort();
        let has_more = keys.len() > limit;
        keys.truncate(limit);
        let next_cursor = has_more.then(|| keys.last().unwrap().as_str().to_string());
        Ok(StoreListPage { keys, next_cursor })
    }

    fn local_root(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

pub(crate) fn open_read_only_no_follow(path: &Path, label: &str) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|error| format!("open {}: {}", label, error))
}

fn checked_existing_object_path(root: &Path, key: &ObjectKey) -> Result<Option<PathBuf>, String> {
    let mut cursor = root.to_path_buf();
    for component in key.as_str().split('/') {
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("symlink traversal in object key '{}'", key))
            }
            Ok(meta) if cursor != root.join(key.as_str()) && !meta.is_dir() => {
                return Err(format!("non-directory ancestor in object key '{}'", key))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("inspect object '{}': {}", key, e)),
        }
    }
    let canonical_parent = fs::canonicalize(
        cursor
            .parent()
            .ok_or_else(|| format!("object '{}' has no parent", key))?,
    )
    .map_err(|e| format!("resolve object parent '{}': {}", key, e))?;
    if !canonical_parent.starts_with(root) {
        return Err(format!("object '{}' escapes local store", key));
    }
    Ok(Some(cursor))
}

fn checked_create_object_path(root: &Path, key: &ObjectKey) -> Result<PathBuf, String> {
    let parts = key.as_str().split('/').collect::<Vec<_>>();
    let mut cursor = root.to_path_buf();
    for component in &parts[..parts.len() - 1] {
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(format!("unsafe object ancestor '{}'", cursor.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&cursor)
                .map_err(|e| format!("create object directory '{}': {}", cursor.display(), e))?,
            Err(e) => {
                return Err(format!(
                    "inspect object directory '{}': {}",
                    cursor.display(),
                    e
                ))
            }
        }
    }
    let canonical_parent = fs::canonicalize(&cursor)
        .map_err(|e| format!("resolve object directory '{}': {}", cursor.display(), e))?;
    if !canonical_parent.starts_with(root) {
        return Err(format!("object '{}' escapes local store", key));
    }
    cursor.push(parts.last().unwrap());
    if let Ok(meta) = fs::symlink_metadata(&cursor) {
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(format!("object target '{}' is unsafe", cursor.display()));
        }
    }
    Ok(cursor)
}

fn normalized_relative_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("'{}' is outside '{}';", path.display(), root.display()))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| format!("non-UTF-8 store path '{}';", path.display()))?,
            ),
            _ => return Err(format!("unsafe store path '{}';", path.display())),
        }
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::super::MAX_STORAGE_MARKER_BYTES;
    use super::*;

    #[test]
    fn local_store_enforces_immutable_objects_and_head_cas() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::open(temp.path().join("store")).unwrap();
        let key = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{:032x}/_uploads/upload-a/files/project/AGENTS.md",
            1
        ))
        .unwrap();
        assert_eq!(
            store.put_immutable(&key, b"one").unwrap(),
            ImmutablePutOutcome::Written
        );
        assert_eq!(
            store.put_immutable(&key, b"one").unwrap(),
            ImmutablePutOutcome::AlreadyPresent
        );
        assert!(store.put_immutable(&key, b"two").is_err());

        let head =
            ObjectKey::parse(format!(".mallard/v1/repositories/{:032x}/_head.json", 1)).unwrap();
        let first = store
            .compare_and_swap(&head, &CasExpectation::Absent, b"first")
            .unwrap();
        let CasOutcome::Written { etag } = first else {
            panic!("initial CAS did not write")
        };
        assert!(matches!(
            store
                .compare_and_swap(&head, &CasExpectation::Absent, b"lost")
                .unwrap(),
            CasOutcome::Conflict { .. }
        ));
        assert!(matches!(
            store
                .compare_and_swap(&head, &CasExpectation::Match(etag), b"second")
                .unwrap(),
            CasOutcome::Written { .. }
        ));
        assert_eq!(
            store
                .get_metadata(&head, MAX_STORAGE_MARKER_BYTES)
                .unwrap()
                .unwrap()
                .bytes,
            b"second"
        );
    }

    #[test]
    fn repository_listing_never_returns_library_objects() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalObjectStore::open(temp.path().join("store")).unwrap();
        let repository_key = ObjectKey::parse(format!(
            ".mallard/v1/repositories/{:032x}/_objects/{}",
            5,
            "c".repeat(64)
        ))
        .unwrap();
        store.put_immutable(&repository_key, b"repo").unwrap();
        let library_key = super::super::library_object_key(
            "library-0123456789abcdef0123456789abcdef",
            "_head.json",
        )
        .unwrap();
        store
            .compare_and_swap(&library_key, &CasExpectation::Absent, b"library")
            .unwrap();

        let page = store
            .list(
                &ObjectPrefix::parse(super::super::REPOSITORY_PREFIX).unwrap(),
                None,
                100,
            )
            .unwrap();
        assert_eq!(page.keys, vec![repository_key]);
    }
}
