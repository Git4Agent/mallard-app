//! Exact, bounded local deletion. Files are checked individually; directories
//! are removed only when empty. No recursive delete operation is used.
use super::domain::{
    validate_portable_relative_path, CatalogRemovalEntry, MAX_CATALOG_FILES,
    MAX_CATALOG_FILE_BYTES, MAX_REVISION_PAYLOAD_BYTES,
};
use mallard_domain::sha256_hex;
use std::collections::BTreeMap;
#[cfg(any(test, not(unix)))]
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;

type Snapshot = BTreeMap<String, CatalogRemovalEntry>;

#[cfg(unix)]
fn open(path: &Path, directory: bool) -> Result<File, String> {
    // Start at the filesystem root: even an ancestor swapped for a symlink
    // between review and apply must not redirect a read or an unlink.
    let components = path
        .strip_prefix("/")
        .map_err(|_| "removal path must be absolute")?
        .iter()
        .map(|s| s.to_str().ok_or("removal path is not UTF-8"))
        .collect::<Result<Vec<_>, _>>()?;
    super::provider_state::open_state_path(Path::new("/"), &components, directory)?
        .ok_or_else(|| "Local files changed. Review removal again.".to_string())
}

#[cfg(windows)]
fn windows_handle(path: &Path, directory: bool, deleting: bool) -> Result<File, String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::*;
    let file = fs::OpenOptions::new()
        .access_mode(FILE_GENERIC_READ | if deleting { DELETE } else { 0 })
        // No writer or rename may replace a reviewed entry while this handle is open.
        .share_mode(FILE_SHARE_READ)
        .custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
        )
        .open(path)
        .map_err(|_| "Cannot lock the local removal entry")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect local removal entry")?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err("Removal refuses reparse points and special files".to_string());
    }
    Ok(file)
}

#[cfg(windows)]
fn windows_parents(path: &Path) -> Result<Vec<File>, String> {
    if !path.is_absolute() {
        return Err("Removal path must be absolute".to_string());
    }
    let mut parents = path.ancestors().skip(1).collect::<Vec<_>>();
    parents.reverse();
    parents
        .into_iter()
        .map(|p| windows_handle(p, true, false))
        .collect()
}

#[cfg(windows)]
fn open(path: &Path, directory: bool) -> Result<File, String> {
    let _parents = windows_parents(path)?;
    windows_handle(path, directory, false)
}

fn digest(file: &File) -> Result<(String, u64), String> {
    let before = file.metadata().map_err(|_| "Cannot inspect removal file")?;
    if !before.is_file() || before.len() > MAX_CATALOG_FILE_BYTES {
        return Err("Removal requires bounded regular files".to_string());
    }
    let mut bytes = Vec::new();
    file.take(MAX_CATALOG_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read removal file")?;
    let after = file.metadata().map_err(|_| "Cannot inspect removal file")?;
    if bytes.len() as u64 != before.len()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err("Local file changed during removal review".to_string());
    }
    Ok((sha256_hex(&bytes), before.len()))
}

#[cfg(unix)]
fn children(path: &Path) -> Result<Vec<(String, bool)>, String> {
    use std::ffi::CStr;
    use std::os::fd::IntoRawFd;
    let fd = open(path, true)?.into_raw_fd();
    // SAFETY: fd is an owned directory descriptor; fdopendir owns it on success.
    let ptr = unsafe { libc::fdopendir(fd) };
    if ptr.is_null() {
        unsafe {
            libc::close(fd);
        }
        return Err("Cannot read removal folder".to_string());
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    let dir = Directory(ptr);
    let mut result = Vec::new();
    loop {
        #[cfg(target_os = "macos")]
        unsafe {
            *libc::__error() = 0;
        }
        #[cfg(target_os = "linux")]
        unsafe {
            *libc::__errno_location() = 0;
        }
        // SAFETY: dir owns the stream; entries are consumed before readdir repeats.
        let entry = unsafe { libc::readdir(dir.0) };
        if entry.is_null() {
            if std::io::Error::last_os_error().raw_os_error().unwrap_or(0) != 0 {
                return Err("Cannot enumerate removal folder".to_string());
            }
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        if result.len() >= MAX_CATALOG_FILES {
            return Err("Removal exceeds its entry limit".to_string());
        }
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: fd is live, name is NUL-terminated, and stat is writable.
        if unsafe {
            libc::fstatat(
                fd,
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err("Removal folder changed".to_string());
        }
        let stat = unsafe { stat.assume_init() };
        let kind = stat.st_mode & libc::S_IFMT;
        if kind != libc::S_IFDIR && kind != libc::S_IFREG {
            return Err("Removal refuses symlinks and special files".to_string());
        }
        result.push((
            name.to_str()
                .map_err(|_| "Removal file name is not UTF-8")?
                .to_string(),
            kind == libc::S_IFDIR,
        ));
    }
    Ok(result)
}

#[cfg(windows)]
fn children(path: &Path) -> Result<Vec<(String, bool)>, String> {
    let _parents = windows_parents(path)?;
    let _directory = windows_handle(path, true, false)?;
    let mut result = Vec::new();
    for entry in fs::read_dir(path).map_err(|_| "Cannot read removal folder")? {
        let entry = entry.map_err(|_| "Cannot read removal entry")?;
        let kind = entry
            .file_type()
            .map_err(|_| "Cannot inspect removal entry")?;
        // The next open checks reparse attributes as well as the reported type.
        let _entry = windows_handle(&entry.path(), kind.is_dir(), false)?;
        if result.len() >= MAX_CATALOG_FILES {
            return Err("Removal exceeds its entry limit".to_string());
        }
        result.push((
            entry
                .file_name()
                .into_string()
                .map_err(|_| "Removal name is not UTF-8")?,
            kind.is_dir(),
        ));
    }
    Ok(result)
}

pub(super) fn snapshot(path: &Path) -> Result<Snapshot, String> {
    let mut result = BTreeMap::from([(String::new(), CatalogRemovalEntry { sha256: None })]);
    let mut pending = vec![String::new()];
    let mut total = 0u64;
    while let Some(parent) = pending.pop() {
        for (name, directory) in children(&path.join(&parent))? {
            let relative = if parent.is_empty() {
                name
            } else {
                format!("{parent}/{name}")
            };
            validate_portable_relative_path(&relative)?;
            if result.len() >= MAX_CATALOG_FILES {
                return Err("Removal exceeds its entry limit".to_string());
            }
            let sha256 = if directory {
                pending.push(relative.clone());
                None
            } else {
                let (hash, bytes) = digest(&open(&path.join(&relative), false)?)?;
                total = total.saturating_add(bytes);
                if total > MAX_REVISION_PAYLOAD_BYTES {
                    return Err("Removal exceeds its byte limit".to_string());
                }
                Some(hash)
            };
            result.insert(relative, CatalogRemovalEntry { sha256 });
        }
    }
    Ok(result)
}

#[cfg(unix)]
fn unlink(path: &Path, expected: &CatalogRemovalEntry) -> Result<(), String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;
    let parent = open(path.parent().ok_or("Missing removal parent")?, true)?;
    let name = CString::new(
        path.file_name()
            .ok_or("Missing removal name")?
            .as_encoded_bytes(),
    )
    .map_err(|_| "Invalid removal name")?;
    let directory = expected.sha256.is_none();
    let flags = libc::O_RDONLY
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | libc::O_CLOEXEC
        | if directory { libc::O_DIRECTORY } else { 0 };
    // SAFETY: parent owns the fd and name is one NUL-terminated component.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err("Removal target changed. Review again.".to_string());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let meta = file
        .metadata()
        .map_err(|_| "Cannot inspect removal target")?;
    if let Some(expected) = &expected.sha256 {
        if digest(&file)?.0 != *expected {
            return Err("File changed. Removal stopped.".to_string());
        }
    }
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err("Removal target changed".to_string());
    }
    let stat = unsafe { stat.assume_init() };
    if stat.st_dev as u64 != meta.dev() || stat.st_ino as u64 != meta.ino() {
        return Err("Removal target was replaced".to_string());
    }
    // unlinkat never follows the final component. AT_REMOVEDIR refuses a
    // nonempty directory, including one with an unreviewed new file.
    if unsafe {
        libc::unlinkat(
            parent.as_raw_fd(),
            name.as_ptr(),
            if directory { libc::AT_REMOVEDIR } else { 0 },
        )
    } != 0
    {
        return Err("Could not remove the checked entry. The folder may have changed.".to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn unlink(path: &Path, expected: &CatalogRemovalEntry) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
    };
    let _parents = windows_parents(path)?;
    let file = windows_handle(path, expected.sha256.is_none(), true)?;
    if let Some(expected) = &expected.sha256 {
        if digest(&file)?.0 != *expected {
            return Err("File changed. Removal stopped.".to_string());
        }
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: file owns the verified handle; the input is the API's exact struct.
    // Windows refuses this operation for a nonempty directory.
    let success = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            &disposition as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&disposition) as u32,
        )
    };
    if success == 0 {
        return Err("Could not remove the checked entry. The folder may have changed.".to_string());
    }
    Ok(())
}

pub(super) fn remove(path: &Path, expected: &Snapshot) -> Result<(), String> {
    if snapshot(path)? != *expected {
        return Err("Local files changed. Review removal again.".to_string());
    }
    for (relative, entry) in expected.iter().filter(|(_, e)| e.sha256.is_some()) {
        unlink(&path.join(relative), entry)?;
    }
    let mut dirs = expected
        .iter()
        .filter(|(_, e)| e.sha256.is_none())
        .collect::<Vec<_>>();
    dirs.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
    for (relative, entry) in dirs {
        unlink(&path.join(relative), entry)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tree() -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("item");
        fs::create_dir_all(root.join("nested/empty")).unwrap();
        fs::write(root.join("pet.json"), b"manifest").unwrap();
        fs::write(root.join("nested/extra.txt"), b"not in portable capture").unwrap();
        (temp, root)
    }
    #[test]
    fn full_tree_removal_includes_extra_files_and_empty_directories() {
        let (_temp, root) = tree();
        let plan = snapshot(&root).unwrap();
        assert_eq!(plan.len(), 5);
        assert_eq!(plan.values().filter(|e| e.sha256.is_some()).count(), 2);
        remove(&root, &plan).unwrap();
        assert!(!root.exists());
    }
    #[test]
    fn changed_or_added_files_stop_before_deletion() {
        let (_temp, root) = tree();
        let plan = snapshot(&root).unwrap();
        fs::write(root.join("pet.json"), b"changed!").unwrap();
        assert!(remove(&root, &plan).is_err());
        assert!(root.join("nested/extra.txt").exists());
        let plan = snapshot(&root).unwrap();
        fs::write(root.join("new.txt"), b"new").unwrap();
        assert!(remove(&root, &plan).is_err());
        assert!(root.join("pet.json").exists());
    }
    #[test]
    fn unlink_checks_digest_and_refuses_a_nonempty_directory() {
        let (_temp, root) = tree();
        let plan = snapshot(&root).unwrap();
        fs::write(root.join("pet.json"), b"replacement").unwrap();
        assert!(unlink(&root.join("pet.json"), &plan["pet.json"]).is_err());
        assert!(unlink(&root, &plan[""]).is_err());
        assert!(root.join("pet.json").exists());
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_entry_and_swapped_parent_never_delete_outside_files() {
        use std::os::unix::fs::symlink;
        let (temp, root) = tree();
        let outside = temp.path().canonicalize().unwrap().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("extra.txt"), b"not in portable capture").unwrap();
        let plan = snapshot(&root).unwrap();
        fs::rename(root.join("nested"), root.join("original")).unwrap();
        symlink(&outside, root.join("nested")).unwrap();
        assert!(snapshot(&root).is_err());
        assert!(unlink(&root.join("nested/extra.txt"), &plan["nested/extra.txt"]).is_err());
        assert!(outside.join("extra.txt").exists());
    }
    #[test]
    fn too_many_empty_directories_are_bounded() {
        let (_temp, root) = tree();
        for i in 0..MAX_CATALOG_FILES {
            fs::create_dir(root.join(format!("d{i}"))).unwrap();
        }
        assert!(snapshot(&root).is_err());
    }
}
