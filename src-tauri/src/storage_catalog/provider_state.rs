//! Bounded, no-follow reads of provider-owned local state.

#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::Read;
use std::path::Path;

pub(super) const MAX_PLUGIN_STATE_BYTES: u64 = 8 * 1024 * 1024;
pub(super) const MAX_STATE_ENTRIES: usize = 4096;

fn validate_components(components: &[&str]) -> Result<(), String> {
    if components.is_empty()
        || components.iter().any(|part| {
            part.is_empty() || part.contains(['/', '\\', '\0']) || matches!(*part, "." | "..")
        })
    {
        return Err("invalid provider state path".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn open_state_path(
    home: &Path,
    components: &[&str],
    directory: bool,
) -> Result<Option<File>, String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::OpenOptionsExt;

    validate_components(components)?;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(home)
        .map_err(|_| "cannot open provider home without following links".to_string())?;
    for (index, component) in components.iter().enumerate() {
        let component = CString::new(*component).map_err(|_| "invalid provider state path")?;
        let mut flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        if directory || index + 1 < components.len() {
            flags |= libc::O_DIRECTORY;
        }
        // SAFETY: file owns the directory fd; component is one validated C string.
        let fd = unsafe { libc::openat(file.as_raw_fd(), component.as_ptr(), flags) };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(format!(
                "cannot open provider state without following links: {error}"
            ));
        }
        // SAFETY: openat returned a new owned descriptor.
        file = unsafe { File::from_raw_fd(fd) };
    }
    Ok(Some(file))
}

#[cfg(not(unix))]
fn checked_state_path(
    home: &Path,
    components: &[&str],
) -> Result<Option<std::path::PathBuf>, String> {
    validate_components(components)?;
    let mut path = home.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        path.push(component);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("cannot inspect provider state".to_string()),
        };
        if metadata.file_type().is_symlink() || (index + 1 < components.len() && !metadata.is_dir())
        {
            return Err("unsafe provider state path".to_string());
        }
    }
    Ok(Some(path))
}

pub(super) fn read_optional_state_file(
    home: &Path,
    components: &[&str],
) -> Result<Option<Vec<u8>>, String> {
    #[cfg(unix)]
    {
        let Some(file) = open_state_path(home, components, false)? else {
            return Ok(None);
        };
        let before = file
            .metadata()
            .map_err(|_| "cannot inspect provider state")?;
        if !before.is_file() || before.len() > MAX_PLUGIN_STATE_BYTES {
            return Err("provider state is not a bounded regular file".to_string());
        }
        let mut bytes = Vec::new();
        (&file)
            .take(MAX_PLUGIN_STATE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "cannot read provider state")?;
        let after = file
            .metadata()
            .map_err(|_| "cannot inspect provider state")?;
        if bytes.len() as u64 > MAX_PLUGIN_STATE_BYTES
            || bytes.len() as u64 != before.len()
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
        {
            return Err("provider state changed during the read or exceeded its limit".to_string());
        }
        Ok(Some(bytes))
    }
    #[cfg(not(unix))]
    {
        let Some(path) = checked_state_path(home, components)? else {
            return Ok(None);
        };
        super::codec::read_file_no_follow(&path, "provider state", MAX_PLUGIN_STATE_BYTES)
            .map(|(bytes, _)| Some(bytes))
    }
}

pub(super) fn read_optional_json(
    home: &Path,
    components: &[&str],
) -> Result<Option<serde_json::Value>, String> {
    read_optional_state_file(home, components)?
        .map(|bytes| {
            serde_json::from_slice(&bytes).map_err(|_| "invalid provider state JSON".to_string())
        })
        .transpose()
}

/// Enumerate only immediate physical directories, bounded across the complete scan.
/// Symlinks (including a cache's `latest` alias) are never traversed.
pub(super) fn child_directories(
    home: &Path,
    components: &[&str],
    remaining: &mut usize,
) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    #[cfg(unix)]
    {
        use std::ffi::CStr;
        use std::os::fd::IntoRawFd;
        let Some(file) = open_state_path(home, components, true)? else {
            return Ok(names);
        };
        let fd = file.into_raw_fd();
        // SAFETY: fd is an owned directory descriptor. fdopendir takes ownership on success.
        let pointer = unsafe { libc::fdopendir(fd) };
        if pointer.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err("cannot enumerate provider state".to_string());
        }
        struct Directory(*mut libc::DIR);
        impl Drop for Directory {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let directory = Directory(pointer);
        loop {
            // readdir signals an error by returning null with errno set.
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            #[cfg(target_os = "linux")]
            unsafe {
                *libc::__errno_location() = 0;
            }
            // SAFETY: directory remains open, and its next entry is consumed before the next call.
            let entry = unsafe { libc::readdir(directory.0) };
            if entry.is_null() {
                if std::io::Error::last_os_error().raw_os_error().unwrap_or(0) != 0 {
                    return Err("cannot enumerate provider state".to_string());
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            *remaining = remaining
                .checked_sub(1)
                .ok_or("provider state exceeds its entry limit")?;
            let name_text = name
                .to_str()
                .map_err(|_| "provider state name is not UTF-8")?;
            if name_text.starts_with('.') {
                continue;
            }
            let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: fd belongs to the open DIR, name is NUL-terminated, metadata is writable.
            let status = unsafe {
                libc::fstatat(
                    fd,
                    name.as_ptr(),
                    metadata.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if status != 0 {
                return Err("provider state changed during enumeration".to_string());
            }
            let metadata = unsafe { metadata.assume_init() };
            if metadata.st_mode & libc::S_IFMT == libc::S_IFDIR {
                names.push(name_text.to_string());
            }
        }
    }
    #[cfg(not(unix))]
    {
        let Some(path) = checked_state_path(home, components)? else {
            return Ok(names);
        };
        for entry in std::fs::read_dir(path).map_err(|_| "cannot enumerate provider state")? {
            *remaining = remaining
                .checked_sub(1)
                .ok_or("provider state exceeds its entry limit")?;
            let entry = entry.map_err(|_| "cannot enumerate provider state")?;
            let kind = entry
                .file_type()
                .map_err(|_| "cannot inspect provider state entry")?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "provider state name is not UTF-8")?;
            if kind.is_dir() && !kind.is_symlink() && !name.starts_with('.') {
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}
