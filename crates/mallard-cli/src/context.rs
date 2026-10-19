//! What every verb runs against: this machine's state repository, a runtime,
//! the sink that shows progress, and the two resolutions an agent leans on —
//! the project the current folder belongs to, and a storage named by id or
//! by name.
use std::path::Path;
use std::sync::Arc;

use mallard_application::events::SyncEventSink;
use mallard_application::project_sync::service::fs_canonicalize;
use mallard_domain::{
    LocalProjectId, MachineProjectState, ProjectBinding, ProjectStorageLink, StorageId, SyncConfig,
};
use mallard_infrastructure::app_settings::{install_app_settings, load_app_settings};
use mallard_infrastructure::persistence::LocalStateRepository;
use tokio::runtime::Runtime;

use crate::error::CliError;
use crate::events::StderrSink;

pub(crate) const LOCAL_STATE_MESSAGE: &str = "Mallard could not access its local storage configuration. If this command is running in a restricted environment, approve read/write access to Mallard's local state and run the same command again.";

pub(crate) struct CliContext {
    repository: LocalStateRepository,
    runtime: Runtime,
    sink: Arc<dyn SyncEventSink>,
}

impl CliContext {
    pub fn from_user_home(json: bool) -> Result<Self, CliError> {
        let repository = LocalStateRepository::from_user_home()
            .map_err(|_| CliError::persistence(LOCAL_STATE_MESSAGE))?;
        // Tuning values must be live before the first operation; a corrupt
        // settings file falls back to defaults, as the desktop does at launch.
        install_app_settings(&load_app_settings(&repository).unwrap_or_default());
        // The Feishu sign-ins this computer keeps live beside the config, as
        // the desktop installs them at launch; without the store every Feishu
        // storage reads as reconnect.
        let _ = mallard_infrastructure::feishu_connections::install(&repository);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|_| CliError::persistence("Mallard could not start its worker."))?;
        Ok(Self {
            repository,
            runtime,
            sink: Arc::new(StderrSink::new(json)),
        })
    }

    pub fn repository(&self) -> &LocalStateRepository {
        &self.repository
    }

    /// The sink an operation narrates to, for callers that hand it on.
    pub fn sink(&self) -> Arc<dyn SyncEventSink> {
        self.sink.clone()
    }

    /// Run one operation the way the desktop runs a command: on a blocking
    /// thread inside the runtime, so the stores' own `block_on` calls and
    /// `Handle::current()` work, and nothing blocks an executor thread.
    pub fn blocking<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&LocalStateRepository, &dyn SyncEventSink) -> Result<T, CliError>
            + Send
            + 'static,
    ) -> Result<T, CliError> {
        let repository = self.repository.clone();
        let sink = self.sink.clone();
        let worker = self
            .runtime
            .spawn_blocking(move || operation(&repository, sink.as_ref()));
        self.runtime
            .block_on(worker)
            .map_err(|_| CliError::persistence("Mallard's worker stopped unexpectedly."))?
    }

    /// Like [`Self::blocking`], for operations that fail with a service sentence.
    pub fn service<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&LocalStateRepository, &dyn SyncEventSink) -> Result<T, String>
            + Send
            + 'static,
    ) -> Result<T, CliError> {
        self.blocking(move |repository, sink| {
            operation(repository, sink).map_err(CliError::from_service)
        })
    }

    pub fn load_config(&self) -> Result<SyncConfig, CliError> {
        self.repository
            .load_config()
            .map_err(|_| CliError::persistence(LOCAL_STATE_MESSAGE))
    }

    pub fn load_bindings(&self) -> Result<MachineProjectState, CliError> {
        self.repository
            .load_bindings()
            .map_err(|_| CliError::persistence(LOCAL_STATE_MESSAGE))
    }

    /// The project's binding on this computer, which a Push or Pull needs.
    pub fn binding(&self, project: &LocalProjectId) -> Result<ProjectBinding, CliError> {
        self.load_bindings()?
            .bindings
            .into_iter()
            .find(|binding| &binding.local_project_id == project)
            .ok_or_else(|| {
                CliError::invalid_with(
                    "project_not_bound",
                    "The project is not bound to a folder on this computer.",
                )
            })
    }

    /// The project's link to one storage.
    pub fn link(
        &self,
        project: &LocalProjectId,
        storage: &StorageId,
    ) -> Result<ProjectStorageLink, CliError> {
        self.load_config()?
            .links
            .into_iter()
            .find(|link| &link.local_project_id == project && &link.storage_id == storage)
            .ok_or_else(|| {
                CliError::invalid_with(
                    "link_not_found",
                    "The project is not linked to that storage. Run `mallard-cli link add`.",
                )
            })
    }

    /// The project a verb concerns: `selector` is a project id or a path
    /// inside the project; without one, the current folder decides.
    pub fn resolve_project(&self, selector: Option<&str>) -> Result<LocalProjectId, CliError> {
        let config = self.load_config()?;
        if let Some(selector) = selector {
            if let Ok(id) = LocalProjectId::parse(selector) {
                if config
                    .projects
                    .iter()
                    .any(|project| project.local_project_id == id)
                {
                    return Ok(id);
                }
            }
            return self.project_containing(Path::new(selector), &config);
        }
        let cwd = std::env::current_dir().map_err(|_| {
            CliError::invalid_code("cwd_unavailable", "The current folder could not be read.")
        })?;
        self.project_containing(&cwd, &config)
    }

    fn project_containing(
        &self,
        path: &Path,
        config: &SyncConfig,
    ) -> Result<LocalProjectId, CliError> {
        let canonical = fs_canonicalize(path).map_err(|_| {
            CliError::invalid_with(
                "project_not_found",
                "That path does not exist. Pass --project <id|path> or run `mallard-cli project list`.",
            )
        })?;
        let bindings = self
            .repository
            .load_bindings()
            .map_err(|_| CliError::persistence(LOCAL_STATE_MESSAGE))?
            .bindings;
        let registered = |id: &LocalProjectId| {
            config
                .projects
                .iter()
                .any(|project| &project.local_project_id == id)
        };
        for ancestor in canonical.ancestors() {
            let mut matches = bindings.iter().filter(|binding| {
                Path::new(&binding.canonical_project_root) == ancestor
                    && registered(&binding.local_project_id)
            });
            if let Some(binding) = matches.next() {
                if matches.next().is_some() {
                    return Err(CliError::invalid_with(
                        "project_ambiguous",
                        "Several projects are registered for this folder. Pass --project <id>.",
                    ));
                }
                return Ok(binding.local_project_id.clone());
            }
        }
        Err(CliError::invalid_with(
            "project_not_found",
            "No Mallard project contains this folder. Pass --project <id|path> or run `mallard-cli project list`.",
        ))
    }

    /// The storage a verb concerns: `selector` is a storage id or its name;
    /// without one, the project's only link decides.
    pub fn resolve_storage(
        &self,
        project: Option<&LocalProjectId>,
        selector: Option<&str>,
    ) -> Result<StorageId, CliError> {
        let config = self.load_config()?;
        if let Some(selector) = selector {
            if let Ok(id) = StorageId::parse(selector) {
                if config.storages.iter().any(|storage| storage.id == id) {
                    return Ok(id);
                }
            }
            let mut named = config
                .storages
                .iter()
                .filter(|storage| storage.name == selector);
            return match (named.next(), named.next()) {
                (Some(storage), None) => Ok(storage.id.clone()),
                (Some(_), Some(_)) => Err(CliError::invalid_with(
                    "storage_ambiguous",
                    "Several storages have that name. Pass the storage id.",
                )),
                (None, _) => Err(CliError::invalid_with(
                    "storage_not_found",
                    "No storage has that id or name. Run `mallard-cli storage list`.",
                )),
            };
        }
        let Some(project) = project else {
            return Err(CliError::invalid_with(
                "storage_required",
                "Pass --storage <id|name>.",
            ));
        };
        let linked: Vec<&StorageId> = config
            .links
            .iter()
            .filter(|link| &link.local_project_id == project)
            .map(|link| &link.storage_id)
            .collect();
        match linked.as_slice() {
            [storage] => Ok((*storage).clone()),
            [] => Err(CliError::invalid_with(
                "storage_required",
                "The project has no storage link yet. Pass --storage <id|name>, or run `mallard-cli link add`.",
            )),
            many => {
                let names: Vec<String> = many
                    .iter()
                    .map(|id| {
                        config
                            .storages
                            .iter()
                            .find(|storage| &storage.id == *id)
                            .map(|storage| format!("{} [{}]", storage.name, storage.id))
                            .unwrap_or_else(|| id.to_string())
                    })
                    .collect();
                Err(CliError::invalid_with(
                    "storage_ambiguous",
                    format!(
                        "The project is linked to several storages; pass --storage: {}",
                        names.join(", ")
                    ),
                ))
            }
        }
    }
}
