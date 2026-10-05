//! The official macOS app identity shared by launch and refresh.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub(crate) struct MacAppBundle {
    pub app_name: &'static str,
    pub bundle_name: &'static str,
    pub bundle_id: &'static str,
    pub executable_name: &'static str,
}

pub(crate) const OFFICIAL_MAC_APP: MacAppBundle = MacAppBundle {
    app_name: "Codex App",
    bundle_name: "ChatGPT.app",
    bundle_id: "com.openai.codex",
    executable_name: "ChatGPT",
};

#[derive(Clone, Debug)]
pub(crate) struct InstalledMacApp {
    pub specification: MacAppBundle,
    pub bundle_path: PathBuf,
}

impl InstalledMacApp {
    /// The Codex CLI the app ships and resumes sessions with — its own copy,
    /// versioned with the app, never the one on PATH.
    pub(crate) fn bundled_cli(&self) -> PathBuf {
        self.bundle_path.join("Contents/Resources/codex-cli/bin/codex")
    }
}

pub(crate) fn find_installed_mac_app() -> Option<InstalledMacApp> {
    find_installed_mac_app_in(
        Path::new("/Applications"),
        crate::persistence::user_home_dir().as_deref(),
    )
}

pub(crate) fn find_installed_mac_app_in(
    applications: &Path,
    user_home: Option<&Path>,
) -> Option<InstalledMacApp> {
    mac_app_candidate_paths(applications, user_home)
        .into_iter()
        .find_map(|path| validate_mac_app_bundle(&path))
}

pub(crate) fn mac_app_candidate_paths(
    applications: &Path,
    user_home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut paths = vec![applications.join(OFFICIAL_MAC_APP.bundle_name)];
    if let Some(home) = user_home {
        paths.push(home.join("Applications").join(OFFICIAL_MAC_APP.bundle_name));
    }
    paths
}

pub(crate) fn validate_mac_app_bundle(path: &Path) -> Option<InstalledMacApp> {
    if path.file_name()?.to_str()? != OFFICIAL_MAC_APP.bundle_name {
        return None;
    }
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let canonical = fs::canonicalize(path).ok()?;
    if canonical != path {
        return None;
    }
    let plist_path = canonical.join("Contents/Info.plist");
    let plist_metadata = fs::symlink_metadata(&plist_path).ok()?;
    if !plist_metadata.is_file() || plist_metadata.file_type().is_symlink() {
        return None;
    }
    let dictionary = plist::Value::from_file(&plist_path)
        .ok()?
        .into_dictionary()?;
    if dictionary
        .get("CFBundleIdentifier")
        .and_then(plist::Value::as_string)
        != Some(OFFICIAL_MAC_APP.bundle_id)
        || dictionary
            .get("CFBundleExecutable")
            .and_then(plist::Value::as_string)
            != Some(OFFICIAL_MAC_APP.executable_name)
    {
        return None;
    }
    let executable = canonical
        .join("Contents/MacOS")
        .join(OFFICIAL_MAC_APP.executable_name);
    let executable_metadata = fs::symlink_metadata(&executable).ok()?;
    if !executable_metadata.is_file() || executable_metadata.file_type().is_symlink() {
        return None;
    }
    let canonical_executable = fs::canonicalize(&executable).ok()?;
    if !canonical_executable.starts_with(&canonical) {
        return None;
    }
    Some(InstalledMacApp {
        specification: OFFICIAL_MAC_APP,
        bundle_path: canonical,
    })
}
