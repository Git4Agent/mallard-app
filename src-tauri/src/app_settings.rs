//! Tauri-facing tuning-settings commands. The document, validation, and the
//! installed process-wide snapshot live in
//! `mallard_infrastructure::app_settings`; this module adds the command
//! surface and the cache consequence the infrastructure crate cannot see:
//! retained observations bake limits into their contents, and a tuning
//! change touches no session file, so it drops them explicitly.
//! Redaction-only changes preserve them. Saves share the host operation lock.

use crate::project_sync::observation_cache::observation_cache;
use mallard_infrastructure::app_settings::{
    install_app_settings, load_app_settings, save_app_settings, AppSettings,
};
use mallard_infrastructure::persistence::LocalStateRepository;

async fn run_settings_io<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("settings worker failed: {error}"))?
}

/// Non-Tauri core, unit-testable: persist, make live, return the read-back.
fn update_app_settings_core(
    repository: &LocalStateRepository,
    settings: &AppSettings,
    reset_caches: impl FnOnce() -> Result<(), String>,
) -> Result<AppSettings, String> {
    let _operation = repository.acquire_host_operation()?;
    let previous = load_app_settings(repository)?;
    save_app_settings(repository, settings)?;
    install_app_settings(settings);
    let saved = load_app_settings(repository)?;
    if previous.tuning_differs(&saved) {
        reset_caches()?;
    }
    Ok(saved)
}

#[tauri::command]
pub async fn get_app_settings() -> Result<AppSettings, String> {
    run_settings_io(|| load_app_settings(&LocalStateRepository::from_user_home()?)).await
}

#[tauri::command]
pub async fn update_app_settings(settings: AppSettings) -> Result<AppSettings, String> {
    run_settings_io(move || {
        let repository = LocalStateRepository::from_user_home()?;
        update_app_settings_core(&repository, &settings, || {
            observation_cache().reset_and_purge_disk(repository.root())
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::update_app_settings_core;
    use mallard_infrastructure::app_settings::AppSettings;
    use mallard_infrastructure::persistence::LocalStateRepository;

    /// Default values only: the install half writes process-wide atomics and
    /// this binary runs its tests concurrently.
    #[test]
    fn update_core_persists_installs_and_reads_back() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let settings = AppSettings::default();

        let saved = update_app_settings_core(&repository, &settings, || Ok(())).unwrap();
        assert_eq!(saved, settings);
        assert!(temp.path().join(".mallard").join("settings.json").is_file());

        let invalid = AppSettings {
            comparison_workers: 0,
            ..settings
        };
        assert!(update_app_settings_core(&repository, &invalid, || Ok(())).is_err());
    }
    #[test]
    fn export_settings_preserve_caches_and_busy_save_preserves_settings() {
        use mallard_infrastructure::app_settings::{load_app_settings, save_app_settings};
        let temp = tempfile::tempdir().unwrap();
        let repo = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        assert!(
            !load_app_settings(&repo)
                .unwrap()
                .credential_redaction_enabled
        );
        let on = AppSettings {
            credential_redaction_enabled: true,
            push_codex_children: true,
            push_claude_children: true,
            ..AppSettings::default()
        };
        for _ in 0..2 {
            let saved = update_app_settings_core(&repo, &on, || {
                panic!("export-setting save must retain caches")
            })
            .unwrap();
            assert_eq!(saved, on);
        }
        let run = mallard_infrastructure::credential_redaction::shared()
            .begin_run()
            .with_enabled(false);
        {
            let _push = repo.acquire_host_operation().unwrap();
            assert!(
                update_app_settings_core(&repo, &AppSettings::default(), || panic!(
                    "busy save purged caches"
                ))
                .is_err()
            );
            assert_eq!(load_app_settings(&repo).unwrap(), on);
            assert_eq!(
                run.stamp().codec,
                mallard_infrastructure::credential_redaction::REDACTION_OFF_CODEC_V1
            );
        }
        update_app_settings_core(&repo, &AppSettings::default(), || {
            panic!("export-setting save purged caches")
        })
        .unwrap();
        // Seed a different tuning value on disk only. Never install non-default atomics in tests.
        save_app_settings(
            &repo,
            &AppSettings {
                comparison_workers: 2,
                ..AppSettings::default()
            },
        )
        .unwrap();
        let purged = std::cell::Cell::new(false);
        update_app_settings_core(&repo, &AppSettings::default(), || {
            purged.set(true);
            Ok(())
        })
        .unwrap();
        assert!(purged.get());
    }
}
