//! The Feishu connection documents: one schema-1 JSON document per
//! connection under `<local state root>/feishu_connections/`, holding the
//! current refresh token, the date Feishu requires a fresh sign-in by, and
//! whether the last refresh request's answer was lost. The document is the
//! only place a Feishu refresh token lives: never `sync_config.json`, a
//! manifest, a backup, or a log. Bounded, atomic, opened no-follow, owner
//! only, under one lock per local state root.
//!
//! The desktop installs the store once at startup with its local state
//! root; tests install nothing and construct one over a temporary root; the
//! CLI installs none and therefore cannot open a Feishu storage.

use crate::persistence::{read_json_bounded, write_json_atomic, LocalStateRepository};
use mallard_domain::{hex_lower, SecretValue};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const FEISHU_CONNECTION_SCHEMA: u32 = 1;
const MAX_CONNECTION_DOCUMENT_BYTES: u64 = 16 * 1024;
const DIRECTORY: &str = "feishu_connections";
const LOCK_FILE: &str = "feishu_connections.lock";
const CONNECTION_ID_HEX_BYTES: usize = 32;

static CONNECTION_LOCK: Mutex<()> = Mutex::new(());
static INSTALLED_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// A connection's name: 32 lowercase hexadecimal characters minted at
/// sign-in. It names the document and nothing else, and its fixed shape
/// makes it a safe filename.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionId(String);

impl ConnectionId {
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() != CONNECTION_ID_HEX_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(format!(
                "Feishu connection ID must be exactly {CONNECTION_ID_HEX_BYTES} lowercase hexadecimal characters"
            ));
        }
        Ok(Self(value))
    }

    pub fn generate() -> Result<Self, String> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| format!("random connection id: {error}"))?;
        Self::parse(hex_lower(&bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for ConnectionId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ConnectionId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// One connection's refresh chain. `refresh_uncertain` is true from the
/// moment a refresh request is sent until a definitive answer is written,
/// so a lost answer is visible to the next launch. `reauthorize_at` is
/// the Unix time Feishu's 365-day limit runs out, set at sign-in and never
/// extended by a refresh.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FeishuConnection {
    pub schema: u32,
    pub connection_id: ConnectionId,
    pub refresh_token: SecretValue,
    pub reauthorize_at: u64,
    pub refresh_uncertain: bool,
    pub updated_at: u64,
}

impl FeishuConnection {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != FEISHU_CONNECTION_SCHEMA {
            return Err(format!(
                "unsupported Feishu connection schema {}",
                self.schema
            ));
        }
        if self.reauthorize_at == 0 || self.updated_at == 0 {
            return Err("Feishu connection timestamps must be nonzero".to_string());
        }
        Ok(())
    }
}

/// Atomic, no-follow, owner-only access to the connection documents under
/// one local state root.
#[derive(Clone, Debug)]
pub struct FeishuConnectionStore {
    root: PathBuf,
    directory: PathBuf,
}

impl FeishuConnectionStore {
    pub fn new(repository: &LocalStateRepository) -> Self {
        Self::at_root(repository.root())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn at_root(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            directory: root.join(DIRECTORY),
        }
    }

    pub fn load(&self, id: &ConnectionId) -> Result<Option<FeishuConnection>, String> {
        let _guard = self.guard()?;
        self.load_unlocked(id)
    }

    /// Replace the document atomically: the token and the uncertain flag
    /// always land together.
    pub fn save(&self, connection: &FeishuConnection) -> Result<(), String> {
        connection.validate()?;
        let _guard = self.guard()?;
        self.ensure_directory()?;
        let path = self.document_path(&connection.connection_id);
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err("the Feishu connection document is not a regular file".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("inspect the Feishu connection document: {error}")),
        }
        write_json_atomic(&self.root, &path, connection, MAX_CONNECTION_DOCUMENT_BYTES)
            .map_err(|error| self.sanitize(error))
    }

    pub fn remove(&self, id: &ConnectionId) -> Result<bool, String> {
        let _guard = self.guard()?;
        let path = self.document_path(id);
        ensure_no_symlinks(&self.root, &path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err("the Feishu connection document is not a regular file".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(format!("inspect the Feishu connection document: {error}")),
        }
        fs::remove_file(&path)
            .map(|()| true)
            .map_err(|error| format!("remove the Feishu connection document: {error}"))
    }

    fn document_path(&self, id: &ConnectionId) -> PathBuf {
        self.directory.join(format!("{}.json", id.as_str()))
    }

    fn load_unlocked(&self, id: &ConnectionId) -> Result<Option<FeishuConnection>, String> {
        let path = self.document_path(id);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.permissions().mode() & 0o077 != 0 => {
                    return Err(
                        "the Feishu connection document is readable by other users".to_string()
                    );
                }
                _ => {}
            }
        }
        let connection =
            read_json_bounded::<FeishuConnection>(&self.root, &path, MAX_CONNECTION_DOCUMENT_BYTES)
                .map_err(|error| self.sanitize(error))?;
        if let Some(connection) = &connection {
            connection.validate()?;
            if &connection.connection_id != id {
                return Err("the Feishu connection document names another connection".into());
            }
        }
        Ok(connection)
    }

    fn ensure_directory(&self) -> Result<(), String> {
        ensure_no_symlinks(&self.root, &self.directory)?;
        match fs::symlink_metadata(&self.directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err("the Feishu connection directory is not a real directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    let mut builder = fs::DirBuilder::new();
                    builder.mode(0o700);
                    match builder.create(&self.directory) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => {
                            return Err(format!("create the Feishu connection directory: {error}"))
                        }
                    }
                }
                #[cfg(not(unix))]
                match fs::create_dir(&self.directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        return Err(format!("create the Feishu connection directory: {error}"))
                    }
                }
            }
            Err(error) => return Err(format!("inspect the Feishu connection directory: {error}")),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("secure the Feishu connection directory: {error}"))?;
        }
        ensure_no_symlinks(&self.root, &self.directory)
    }

    fn guard(&self) -> Result<ConnectionGuard, String> {
        let process = CONNECTION_LOCK
            .lock()
            .map_err(|_| "the Feishu connection lock is poisoned".to_string())?;
        let path = self.root.join(LOCK_FILE);
        ensure_no_symlinks(&self.root, &path)?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options
            .open(&path)
            .map_err(|error| format!("open the Feishu connection lock: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("inspect the Feishu connection lock: {error}"))?;
        if !metadata.file_type().is_file() {
            return Err("the Feishu connection lock is not a regular file".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.nlink() != 1 {
                return Err("the Feishu connection lock has unsafe hard links".into());
            }
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("secure the Feishu connection lock: {error}"))?;
        }
        file.lock()
            .map_err(|error| format!("lock the Feishu connections: {error}"))?;
        Ok(ConnectionGuard {
            _process: process,
            _file: file,
        })
    }

    /// Persistence errors may name the machine-local state root; keep the
    /// reason without the host-specific path.
    fn sanitize(&self, error: String) -> String {
        error.replace(self.root.to_string_lossy().as_ref(), "<local state>")
    }
}

struct ConnectionGuard {
    _process: std::sync::MutexGuard<'static, ()>,
    _file: fs::File,
}

fn ensure_no_symlinks(root: &Path, destination: &Path) -> Result<(), String> {
    let relative = destination
        .strip_prefix(root)
        .map_err(|_| "the Feishu connection path escapes local state".to_string())?;
    let mut current = root.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            current.push(component.as_os_str());
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("the Feishu connection path traverses a symlink".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("inspect the Feishu connection path: {error}")),
        }
    }
    Ok(())
}

/// Install the process-wide store over this local state root, once. A
/// second install with the same root is a no-op; another root is refused.
pub fn install(repository: &LocalStateRepository) -> Result<(), String> {
    let root = repository.root().to_path_buf();
    match INSTALLED_ROOT.get() {
        Some(installed) if installed == &root => Ok(()),
        Some(_) => Err("a Feishu connection store is already installed for another root".into()),
        None => INSTALLED_ROOT
            .set(root)
            .map_err(|_| "a Feishu connection store is already installed".to_string()),
    }
}

/// The installed store, or nothing in a process that installed none.
pub fn installed() -> Option<FeishuConnectionStore> {
    INSTALLED_ROOT
        .get()
        .map(|root| FeishuConnectionStore::at_root(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID_A: &str = "0123456789abcdef0123456789abcdef";

    fn store(temp: &tempfile::TempDir) -> FeishuConnectionStore {
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        FeishuConnectionStore::new(&repository)
    }

    fn connection(id: &str, token: &str) -> FeishuConnection {
        FeishuConnection {
            schema: FEISHU_CONNECTION_SCHEMA,
            connection_id: ConnectionId::parse(id).unwrap(),
            refresh_token: SecretValue::parse(token).unwrap(),
            reauthorize_at: 1_800_000_000,
            refresh_uncertain: false,
            updated_at: 1_700_000_000,
        }
    }

    #[test]
    fn connection_ids_are_exactly_32_lowercase_hex_and_random() {
        for invalid in [
            "",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "0123456789ABCDEF0123456789abcdef",
            "../../../../../../../etc/passwd0",
        ] {
            assert!(
                ConnectionId::parse(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
        let first = ConnectionId::generate().unwrap();
        let second = ConnectionId::generate().unwrap();
        assert_ne!(first, second);
        assert_eq!(first.as_str().len(), 32);
    }

    #[test]
    fn documents_round_trip_replace_atomically_and_remove() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let id = ConnectionId::parse(ID_A).unwrap();
        assert_eq!(store.load(&id).unwrap(), None);
        let first = connection(ID_A, "refresh-one");
        store.save(&first).unwrap();
        assert_eq!(store.load(&id).unwrap(), Some(first.clone()));
        // Rotation: the token and the flag change in one write.
        let mut rotated = first.clone();
        rotated.refresh_token = SecretValue::parse("refresh-two").unwrap();
        rotated.refresh_uncertain = true;
        rotated.updated_at += 1;
        store.save(&rotated).unwrap();
        let loaded = store.load(&id).unwrap().unwrap();
        assert_eq!(loaded.refresh_token.expose(), "refresh-two");
        assert!(loaded.refresh_uncertain);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.document_path(&id))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
            let directory = fs::metadata(&store.directory).unwrap().permissions().mode() & 0o777;
            assert_eq!(directory, 0o700);
        }
        assert!(store.remove(&id).unwrap());
        assert!(!store.remove(&id).unwrap());
        assert_eq!(store.load(&id).unwrap(), None);
        // The document, never `sync_config.json`: nothing else under the root
        // holds the token.
        assert!(!temp.path().join(".mallard/sync_config.json").exists());
    }

    #[test]
    fn schema_and_id_mismatches_and_unknown_fields_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let id = ConnectionId::parse(ID_A).unwrap();
        let mut wrong_schema = connection(ID_A, "refresh");
        wrong_schema.schema = 2;
        assert!(store.save(&wrong_schema).is_err());
        let mut zero = connection(ID_A, "refresh");
        zero.reauthorize_at = 0;
        assert!(store.save(&zero).is_err());
        store.ensure_directory().unwrap();
        let path = store.document_path(&id);
        let mut value = serde_json::to_value(connection(ID_A, "refresh")).unwrap();
        value["access_token"] = serde_json::json!("must-never-be-stored");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(store.load(&id).is_err());
        let other = connection("ffffffffffffffffffffffffffffffff", "refresh");
        fs::write(&path, serde_json::to_vec(&other).unwrap()).unwrap();
        assert!(store.load(&id).is_err());
    }

    #[test]
    fn oversized_documents_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let id = ConnectionId::parse(ID_A).unwrap();
        store.ensure_directory().unwrap();
        let path = store.document_path(&id);
        fs::write(
            &path,
            vec![b'x'; MAX_CONNECTION_DOCUMENT_BYTES as usize + 1],
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let error = store.load(&id).unwrap_err();
        assert!(!error.contains(temp.path().to_str().unwrap()), "{error}");
        let mut long = connection(ID_A, "refresh");
        long.refresh_token = SecretValue::parse("t".repeat(20_000)).unwrap();
        assert!(store.save(&long).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_group_readable_documents_are_refused() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let id = ConnectionId::parse(ID_A).unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, &store.directory).unwrap();
        assert!(store.save(&connection(ID_A, "refresh")).is_err());
        assert!(store.load(&id).is_err());
        fs::remove_file(&store.directory).unwrap();

        store.ensure_directory().unwrap();
        let outside_file = outside.join("document.json");
        fs::write(
            &outside_file,
            serde_json::to_vec(&connection(ID_A, "refresh")).unwrap(),
        )
        .unwrap();
        symlink(&outside_file, store.document_path(&id)).unwrap();
        assert!(store.load(&id).is_err());
        assert!(store.save(&connection(ID_A, "refresh")).is_err());
        assert!(store.remove(&id).is_err());
        fs::remove_file(store.document_path(&id)).unwrap();

        store.save(&connection(ID_A, "refresh")).unwrap();
        fs::set_permissions(store.document_path(&id), fs::Permissions::from_mode(0o644)).unwrap();
        let error = store.load(&id).unwrap_err();
        assert!(error.contains("readable by other users"), "{error}");
    }

    #[test]
    fn install_is_once_per_root_and_tests_install_nothing() {
        // The lib test binary shares one process: nothing here installs the
        // global store, so an adapter opened in another test finds none.
        assert!(installed().is_none());
    }
}
