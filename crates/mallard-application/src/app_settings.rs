//! Saving tuning settings. The document, validation, and the installed
//! process-wide snapshot live in `mallard_infrastructure::app_settings`; this
//! adds the cache consequence the infrastructure crate cannot see: retained
//! observations bake limits into their contents, and a tuning change touches
//! no session file, so it drops them explicitly. Redaction-only changes
//! preserve them. Saves share the host operation lock.

use crate::project_sync::observation_cache::observation_cache;
use mallard_infrastructure::app_settings::{
    install_app_settings, load_app_settings, save_app_settings, AppSettings,
};
use mallard_infrastructure::persistence::LocalStateRepository;

/// Persist, make live, return the read-back.
pub fn update_app_settings(
    repository: &LocalStateRepository,
    settings: &AppSettings,
) -> Result<AppSettings, String> {
    let _operation = repository.acquire_host_operation()?;
    let previous = load_app_settings(repository)?;
    save_app_settings(repository, settings)?;
    install_app_settings(settings);
    let saved = load_app_settings(repository)?;
    if previous.tuning_differs(&saved) {
        observation_cache().reset_and_purge_disk(repository.root())?;
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::update_app_settings;
    use mallard_infrastructure::app_settings::AppSettings;
    use mallard_infrastructure::persistence::LocalStateRepository;

    /// Default values only: the install half writes process-wide atomics and
    /// this binary runs its tests concurrently.
    #[test]
    fn update_persists_installs_and_reads_back() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let settings = AppSettings::default();

        let saved = update_app_settings(&repository, &settings).unwrap();
        assert_eq!(saved, settings);
        assert!(temp.path().join(".mallard").join("settings.json").is_file());

        let invalid = AppSettings {
            comparison_workers: 0,
            ..settings
        };
        assert!(update_app_settings(&repository, &invalid).is_err());
    }

    #[test]
    fn export_settings_preserve_the_observation_cache_file() {
        use mallard_infrastructure::app_settings::{load_app_settings, save_app_settings};
        let temp = tempfile::tempdir().unwrap();
        let repo = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        assert!(
            !load_app_settings(&repo)
                .unwrap()
                .credential_redaction_enabled
        );
        // A cache file a tuning change would purge; an export-only change
        // leaves it where it is.
        let cache_file = repo.root().join("observation_cache.json");
        std::fs::write(&cache_file, b"{}").unwrap();
        let on = AppSettings {
            credential_redaction_enabled: true,
            push_codex_children: true,
            push_claude_children: true,
            ..AppSettings::default()
        };
        for _ in 0..2 {
            let saved = update_app_settings(&repo, &on).unwrap();
            assert!(saved.credential_redaction_enabled);
            assert!(saved.push_codex_children && saved.push_claude_children);
            assert!(
                cache_file.is_file(),
                "export-setting save must retain caches"
            );
        }
        let _busy = repo.acquire_host_operation().unwrap();
        save_app_settings(&repo, &AppSettings::default()).unwrap();
        assert!(!load_app_settings(&repo).unwrap().push_codex_children);
    }
}
