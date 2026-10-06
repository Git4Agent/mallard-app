//! App-wide settings: the `~/.mallard/settings.json` document and the
//! process-wide tuning snapshot the hot paths read. Export settings are loaded per operation.
//!
//! The document follows the activity-log policy recipe — schema constant,
//! hard-reject validation, atomic write, missing file means defaults — with
//! one deliberate divergence: every field carries a serde default, so a
//! partial or hand-edited file parses instead of failing. Installed tuning values
//! live in process-wide atomics; readers pay one relaxed load, and a process
//! that never installs (tests, the CLI) runs on the documented defaults.

use crate::cli_discovery::MAX_SELECTED_PATH_BYTES;
use crate::persistence::{write_json_atomic, LocalStateRepository};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub const APP_SETTINGS_SCHEMA: u32 = 1;
/// Byte budget for one visible timeline message, one tool argument or result
/// preview, and one expanded tool detail; the page budget is twice this.
pub const DEFAULT_TIMELINE_PREVIEW_LIMIT_BYTES: u64 = 1024 * 1024;
pub const DEFAULT_COMPARISON_WORKERS: u32 = 4;
pub const DEFAULT_CLOUD_TRANSFER_CONCURRENCY: u32 = 6;
const SETTINGS_FILE: &str = "settings.json";
/// Two saved CLI paths of up to 4096 bytes each, JSON-escaped, still fit.
const MAX_SETTINGS_BYTES: u64 = 16 * 1024;

fn default_enabled() -> bool {
    true
}

fn default_schema() -> u32 {
    APP_SETTINGS_SCHEMA
}

fn default_conversation_file_limit_bytes() -> u64 {
    mallard_domain::MAX_CONVERSATION_FILE_BYTES
}

fn default_timeline_preview_limit_bytes() -> u64 {
    DEFAULT_TIMELINE_PREVIEW_LIMIT_BYTES
}

fn default_comparison_workers() -> u32 {
    DEFAULT_COMPARISON_WORKERS
}

fn default_cloud_transfer_concurrency() -> u32 {
    DEFAULT_CLOUD_TRANSFER_CONCURRENCY
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AppSettings {
    /// The Codex executable this device runs, chosen by the user. Absent
    /// means automatic discovery. Never a tuning value and never portable.
    #[serde(default)]
    pub codex_cli_path: Option<String>,
    /// The Claude executable this device runs; see `codex_cli_path`.
    #[serde(default)]
    pub claude_cli_path: Option<String>,
    #[serde(default)]
    pub push_codex_children: bool,
    #[serde(default)]
    pub push_claude_children: bool,
    #[serde(default)]
    pub credential_redaction_enabled: bool,
    /// Whether the desktop app asks GitHub for the latest published release
    /// on its own schedule. A manual check ignores this. Not a tuning value:
    /// toggling it must never purge caches.
    #[serde(default = "default_enabled")]
    pub update_check_enabled: bool,
    #[serde(default = "default_schema")]
    pub schema: u32,
    #[serde(default = "default_conversation_file_limit_bytes")]
    pub conversation_file_limit_bytes: u64,
    #[serde(default = "default_timeline_preview_limit_bytes")]
    pub timeline_preview_limit_bytes: u64,
    #[serde(default = "default_comparison_workers")]
    pub comparison_workers: u32,
    #[serde(default = "default_cloud_transfer_concurrency")]
    pub cloud_transfer_concurrency: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema: APP_SETTINGS_SCHEMA,
            codex_cli_path: None,
            claude_cli_path: None,
            push_codex_children: false,
            push_claude_children: false,
            credential_redaction_enabled: false,
            update_check_enabled: true,
            conversation_file_limit_bytes: default_conversation_file_limit_bytes(),
            timeline_preview_limit_bytes: DEFAULT_TIMELINE_PREVIEW_LIMIT_BYTES,
            comparison_workers: DEFAULT_COMPARISON_WORKERS,
            cloud_transfer_concurrency: DEFAULT_CLOUD_TRANSFER_CONCURRENCY,
        }
    }
}

impl AppSettings {
    pub fn wire_children(
        &self,
        provider: &mallard_domain::ProviderKey,
    ) -> mallard_domain::ConversationWireChildren {
        let enabled = match provider.as_str() {
            "codex" => self.push_codex_children,
            "claude" => self.push_claude_children,
            _ => false,
        };
        if enabled {
            mallard_domain::ConversationWireChildren::On
        } else {
            mallard_domain::ConversationWireChildren::Off
        }
    }

    /// The CLI executable the user saved for `provider`, if any.
    pub fn cli_path(&self, provider: &mallard_domain::ProviderKey) -> Option<&Path> {
        match provider.as_str() {
            "codex" => self.codex_cli_path.as_deref(),
            "claude" => self.claude_cli_path.as_deref(),
            _ => None,
        }
        .map(Path::new)
    }

    /// Tuning values only: a CLI path or an export setting never purges a cache.
    pub fn tuning_differs(&self, other: &Self) -> bool {
        self.conversation_file_limit_bytes != other.conversation_file_limit_bytes
            || self.timeline_preview_limit_bytes != other.timeline_preview_limit_bytes
            || self.comparison_workers != other.comparison_workers
            || self.cloud_transfer_concurrency != other.cloud_transfer_concurrency
    }

    /// The preview ceiling is load-bearing, not taste: page content budgets
    /// twice the preview, the page check runs before an event is added, and
    /// JSON escaping can double the bytes again — 2 MB is the largest value
    /// whose worst page stays inside the fixed 16 MB response bound.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != APP_SETTINGS_SCHEMA {
            return Err(format!("unsupported settings schema {}", self.schema));
        }
        if !(1024 * 1024..=512 * 1024 * 1024).contains(&self.conversation_file_limit_bytes) {
            return Err("thread size limit must be between 1 MB and 512 MB".to_string());
        }
        if !(64 * 1024..=2 * 1024 * 1024).contains(&self.timeline_preview_limit_bytes) {
            return Err("preview size limit must be between 64 KB and 2 MB".to_string());
        }
        if !(1..=16).contains(&self.comparison_workers) {
            return Err("comparison workers must be between 1 and 16".to_string());
        }
        if !(1..=16).contains(&self.cloud_transfer_concurrency) {
            return Err("cloud transfer concurrency must be between 1 and 16".to_string());
        }
        for (label, path) in [
            ("Codex", &self.codex_cli_path),
            ("Claude", &self.claude_cli_path),
        ] {
            if let Some(path) = path {
                validate_cli_path(label, path)?;
            }
        }
        Ok(())
    }
}

/// One absolute file path: no arguments, no shell syntax, no control
/// characters. Spaces and Unicode are ordinary path content.
fn validate_cli_path(label: &str, path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > MAX_SELECTED_PATH_BYTES {
        return Err(format!(
            "{label} CLI path must be between 1 and {MAX_SELECTED_PATH_BYTES} bytes"
        ));
    }
    if path.chars().any(char::is_control) {
        return Err(format!("{label} CLI path contains control characters"));
    }
    if !Path::new(path).is_absolute() {
        return Err(format!("{label} CLI path must be absolute"));
    }
    Ok(())
}

fn settings_path(repository: &LocalStateRepository) -> PathBuf {
    repository.root().join(SETTINGS_FILE)
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("settings path '{}' is a symlink", path.display()))
        }
        _ => Ok(()),
    }
}

/// Missing file means defaults; a present file must parse (missing fields
/// fall back per field) and validate.
pub fn load_app_settings(repository: &LocalStateRepository) -> Result<AppSettings, String> {
    let path = settings_path(repository);
    reject_symlink(&path)?;
    let settings: AppSettings = match fs::metadata(&path) {
        Ok(metadata) if metadata.len() > MAX_SETTINGS_BYTES => {
            return Err(format!(
                "settings '{}' exceed {MAX_SETTINGS_BYTES} bytes",
                path.display()
            ))
        }
        Ok(_) => {
            let bytes = fs::read(&path)
                .map_err(|error| format!("read settings '{}': {error}", path.display()))?;
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("parse settings '{}': {error}", path.display()))?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AppSettings::default(),
        Err(error) => return Err(format!("read settings '{}': {error}", path.display())),
    };
    settings.validate()?;
    Ok(settings)
}

pub fn save_app_settings(
    repository: &LocalStateRepository,
    settings: &AppSettings,
) -> Result<(), String> {
    settings.validate()?;
    let path = settings_path(repository);
    reject_symlink(&path)?;
    write_json_atomic(repository.root(), &path, settings, MAX_SETTINGS_BYTES)
}

static CONVERSATION_FILE_LIMIT_BYTES: AtomicU64 =
    AtomicU64::new(mallard_domain::MAX_CONVERSATION_FILE_BYTES);
static TIMELINE_PREVIEW_LIMIT_BYTES: AtomicU64 =
    AtomicU64::new(DEFAULT_TIMELINE_PREVIEW_LIMIT_BYTES);
static COMPARISON_WORKERS: AtomicU32 = AtomicU32::new(DEFAULT_COMPARISON_WORKERS);
static CLOUD_TRANSFER_CONCURRENCY: AtomicU32 = AtomicU32::new(DEFAULT_CLOUD_TRANSFER_CONCURRENCY);

/// Make `settings` the values every accessor below returns, process-wide.
/// The four cells are independent; a reader racing an install may briefly mix
/// two settings generations. Every consumer tolerates the mix because each
/// gate is correct under either value.
pub fn install_app_settings(settings: &AppSettings) {
    CONVERSATION_FILE_LIMIT_BYTES.store(settings.conversation_file_limit_bytes, Ordering::Relaxed);
    TIMELINE_PREVIEW_LIMIT_BYTES.store(settings.timeline_preview_limit_bytes, Ordering::Relaxed);
    COMPARISON_WORKERS.store(settings.comparison_workers, Ordering::Relaxed);
    CLOUD_TRANSFER_CONCURRENCY.store(settings.cloud_transfer_concurrency, Ordering::Relaxed);
}

/// Largest session transcript any path may read, compare, push, or pull.
pub fn conversation_file_limit_bytes() -> u64 {
    CONVERSATION_FILE_LIMIT_BYTES.load(Ordering::Relaxed)
}

/// Maximum UTF-8 bytes returned for one visible timeline message.
pub fn timeline_message_preview_bytes() -> usize {
    TIMELINE_PREVIEW_LIMIT_BYTES.load(Ordering::Relaxed) as usize
}

/// Maximum UTF-8 bytes returned for either the arguments or result preview of
/// one timeline tool call. The fields are bounded independently.
pub fn timeline_tool_preview_bytes() -> usize {
    TIMELINE_PREVIEW_LIMIT_BYTES.load(Ordering::Relaxed) as usize
}

/// Maximum combined UTF-8 bytes returned by an expanded tool-call detail.
pub fn timeline_tool_detail_bytes() -> usize {
    TIMELINE_PREVIEW_LIMIT_BYTES.load(Ordering::Relaxed) as usize
}

/// Maximum combined message, argument-preview, and result-preview bytes in
/// one timeline page: twice the preview budget, preserving the shipped ratio.
pub fn timeline_page_content_bytes() -> usize {
    (TIMELINE_PREVIEW_LIMIT_BYTES.load(Ordering::Relaxed) as usize).saturating_mul(2)
}

/// Upper bound on parallel local scan workers; item count and
/// `available_parallelism` still apply on top.
pub fn comparison_worker_cap() -> usize {
    COMPARISON_WORKERS.load(Ordering::Relaxed) as usize
}

/// Parallel object transfers per push or pull: both the bulk semaphore width
/// and the batch size, which only take effect together.
pub fn cloud_transfer_concurrency() -> usize {
    CLOUD_TRANSFER_CONCURRENCY.load(Ordering::Relaxed) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> (tempfile::TempDir, LocalStateRepository) {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        (temp, repository)
    }

    #[test]
    fn defaults_equal_the_constants_the_knobs_replaced() {
        let settings = AppSettings::default();
        assert_eq!(settings.schema, 1);
        assert!(!settings.push_codex_children);
        assert!(!settings.push_claude_children);
        assert!(!settings.credential_redaction_enabled);
        assert_eq!(
            settings.conversation_file_limit_bytes,
            mallard_domain::MAX_CONVERSATION_FILE_BYTES
        );
        assert_eq!(settings.timeline_preview_limit_bytes, 1024 * 1024);
        assert_eq!(settings.comparison_workers, 4);
        assert_eq!(settings.cloud_transfer_concurrency, 6);
        settings.validate().unwrap();
    }

    #[test]
    fn validate_rejects_each_field_outside_its_range() {
        let ok = AppSettings::default();
        for (label, bad) in [
            (
                "schema",
                AppSettings {
                    schema: 2,
                    ..ok.clone()
                },
            ),
            (
                "thread floor",
                AppSettings {
                    conversation_file_limit_bytes: 1024 * 1024 - 1,
                    ..ok.clone()
                },
            ),
            (
                "thread ceiling",
                AppSettings {
                    conversation_file_limit_bytes: 512 * 1024 * 1024 + 1,
                    ..ok.clone()
                },
            ),
            (
                "preview floor",
                AppSettings {
                    timeline_preview_limit_bytes: 64 * 1024 - 1,
                    ..ok.clone()
                },
            ),
            (
                "preview ceiling",
                AppSettings {
                    timeline_preview_limit_bytes: 2 * 1024 * 1024 + 1,
                    ..ok.clone()
                },
            ),
            (
                "workers floor",
                AppSettings {
                    comparison_workers: 0,
                    ..ok.clone()
                },
            ),
            (
                "workers ceiling",
                AppSettings {
                    comparison_workers: 17,
                    ..ok.clone()
                },
            ),
            (
                "transfers floor",
                AppSettings {
                    cloud_transfer_concurrency: 0,
                    ..ok.clone()
                },
            ),
            (
                "transfers ceiling",
                AppSettings {
                    cloud_transfer_concurrency: 17,
                    ..ok.clone()
                },
            ),
        ] {
            assert!(bad.validate().is_err(), "{label} must be rejected");
        }
        AppSettings {
            conversation_file_limit_bytes: 512 * 1024 * 1024,
            timeline_preview_limit_bytes: 2 * 1024 * 1024,
            comparison_workers: 16,
            cloud_transfer_concurrency: 16,
            ..ok.clone()
        }
        .validate()
        .unwrap();
        AppSettings {
            conversation_file_limit_bytes: 1024 * 1024,
            timeline_preview_limit_bytes: 64 * 1024,
            comparison_workers: 1,
            cloud_transfer_concurrency: 1,
            ..ok.clone()
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn missing_file_loads_the_defaults() {
        let (_temp, repository) = repository();
        assert_eq!(
            load_app_settings(&repository).unwrap(),
            AppSettings::default()
        );
    }

    #[test]
    fn save_then_load_round_trips_and_rejects_invalid() {
        let (_temp, repository) = repository();
        let settings = AppSettings {
            push_codex_children: true,
            push_claude_children: true,
            credential_redaction_enabled: true,
            conversation_file_limit_bytes: 200 * 1024 * 1024,
            timeline_preview_limit_bytes: 512 * 1024,
            comparison_workers: 8,
            cloud_transfer_concurrency: 2,
            ..AppSettings::default()
        };
        save_app_settings(&repository, &settings).unwrap();
        assert_eq!(load_app_settings(&repository).unwrap(), settings);

        let invalid = AppSettings {
            comparison_workers: 0,
            ..settings.clone()
        };
        assert!(save_app_settings(&repository, &invalid).is_err());
        // The rejected save must not have touched the stored document.
        assert_eq!(load_app_settings(&repository).unwrap(), settings);
    }

    #[test]
    fn partial_documents_fill_missing_fields_from_defaults() {
        let (_temp, repository) = repository();
        std::fs::write(
            repository.root().join(super::SETTINGS_FILE),
            b"{\"schema\":1,\"comparison_workers\":8}\n",
        )
        .unwrap();
        let settings = load_app_settings(&repository).unwrap();
        assert_eq!(settings.comparison_workers, 8);
        assert_eq!(
            settings,
            AppSettings {
                comparison_workers: 8,
                ..AppSettings::default()
            }
        );
    }

    #[test]
    fn child_switches_default_off_and_round_trip_independently() {
        let (_temp, repo) = repository();
        let defaults: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(!defaults.push_codex_children);
        assert!(!defaults.push_claude_children);
        assert!(!defaults.credential_redaction_enabled);
        for (codex, claude) in [(false, true), (true, false), (false, false), (true, true)] {
            let settings = AppSettings {
                push_codex_children: codex,
                push_claude_children: claude,
                ..defaults.clone()
            };
            save_app_settings(&repo, &settings).unwrap();
            assert_eq!(load_app_settings(&repo).unwrap(), settings);
            assert!(!settings.tuning_differs(&defaults));
        }
        for invalid in [
            r#"{"push_codex_children":null}"#,
            r#"{"push_claude_children":"on"}"#,
        ] {
            assert!(serde_json::from_str::<AppSettings>(invalid).is_err());
        }
    }

    #[test]
    fn out_of_range_stored_values_fail_the_load() {
        let (_temp, repository) = repository();
        std::fs::write(
            repository.root().join(super::SETTINGS_FILE),
            b"{\"schema\":1,\"cloud_transfer_concurrency\":64}\n",
        )
        .unwrap();
        assert!(load_app_settings(&repository).is_err());
    }

    /// Installing the defaults must be observable through every accessor.
    /// Deliberately the only test that calls `install_app_settings`, and only
    /// with default values: the atomics are process-wide and the test binary
    /// runs its tests concurrently.
    #[test]
    fn accessors_serve_the_installed_snapshot() {
        install_app_settings(&AppSettings::default());
        assert_eq!(
            conversation_file_limit_bytes(),
            mallard_domain::MAX_CONVERSATION_FILE_BYTES
        );
        assert_eq!(timeline_message_preview_bytes(), 1024 * 1024);
        assert_eq!(timeline_tool_preview_bytes(), 1024 * 1024);
        assert_eq!(timeline_tool_detail_bytes(), 1024 * 1024);
        assert_eq!(timeline_page_content_bytes(), 2 * 1024 * 1024);
        assert_eq!(comparison_worker_cap(), 4);
        assert_eq!(cloud_transfer_concurrency(), 6);
    }
}
