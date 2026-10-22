//! Provider maintenance actions. Adapters declare actions and this registry
//! dispatches them by stable provider and action keys.

use mallard_domain::{ActionKey, BoundedConfigObject, ProviderKey};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::providers::{AgentProviderRegistry, MaintenanceActionDescriptor};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// The generic request every maintenance action receives.
#[derive(Deserialize, Clone, Debug)]
pub struct MaintenanceActionRequest {
    pub provider: ProviderKey,
    pub action_id: ActionKey,
    /// Adapter-typed arguments, for example the ones an after-Pull offer
    /// handed out.
    #[serde(default)]
    pub arguments: BoundedConfigObject,
    /// The user confirmed the action's declared confirmation prompt.
    #[serde(default)]
    pub confirmed: bool,
}

/// The common result returned by a provider maintenance action.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct MaintenanceActionResult {
    pub provider: ProviderKey,
    pub action_id: ActionKey,
    pub ready: bool,
    pub summary: String,
    /// What a finished action did, for the interface to translate `summary`
    /// by, with `app_name` and `home_path` as its values. A failed action's
    /// summary is an error and carries no code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_path: Option<String>,
    #[serde(default)]
    pub facts: BoundedConfigObject,
}

pub(super) trait MaintenanceExecutor: Send + Sync {
    fn execute(
        &self,
        repository: &LocalStateRepository,
        request: &MaintenanceActionRequest,
    ) -> Result<MaintenanceActionResult, String>;
}

pub(super) struct Registration {
    pub(super) executors: Vec<(ProviderKey, ActionKey, Arc<dyn MaintenanceExecutor>)>,
}

pub struct MaintenanceRegistry {
    providers: &'static AgentProviderRegistry,
    executors: BTreeMap<(ProviderKey, ActionKey), Arc<dyn MaintenanceExecutor>>,
}

impl MaintenanceRegistry {
    fn from_registrations(
        providers: &'static AgentProviderRegistry,
        registrations: Vec<Registration>,
    ) -> Result<Self, String> {
        let mut executors: BTreeMap<(ProviderKey, ActionKey), Arc<dyn MaintenanceExecutor>> =
            BTreeMap::new();
        for registration in registrations {
            for (provider, action_id, executor) in registration.executors {
                if executors
                    .insert((provider.clone(), action_id.clone()), executor)
                    .is_some()
                {
                    return Err(format!(
                        "maintenance action '{}' for '{}' is registered twice",
                        action_id, provider
                    ));
                }
            }
        }
        Ok(Self {
            providers,
            executors,
        })
    }

    pub fn production() -> &'static MaintenanceRegistry {
        static REGISTRY: OnceLock<MaintenanceRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            MaintenanceRegistry::from_registrations(
                AgentProviderRegistry::production(),
                vec![super::codex_maintenance::registration()],
            )
            .expect("production maintenance registry has no duplicate registrations")
        })
    }

    /// The production registrations plus the test-only provider action.
    #[cfg(test)]
    pub fn with_test_provider() -> &'static MaintenanceRegistry {
        static REGISTRY: OnceLock<MaintenanceRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            MaintenanceRegistry::from_registrations(
                Box::leak(Box::new(AgentProviderRegistry::with_test_provider())),
                vec![
                    super::codex_maintenance::registration(),
                    test_provider::registration(),
                ],
            )
            .expect("test maintenance registry has no duplicate registrations")
        })
    }

    pub fn descriptor(
        &self,
        provider: &ProviderKey,
        action_id: &ActionKey,
    ) -> Option<MaintenanceActionDescriptor> {
        self.providers
            .get(provider)?
            .maintenance_actions()
            .into_iter()
            .find(|action| &action.action_id == action_id)
    }

    /// Execute one declared action. The adapter must declare it, this build
    /// must register an executor, and a confirming action must be confirmed.
    pub fn execute(
        &self,
        repository: &LocalStateRepository,
        request: &MaintenanceActionRequest,
    ) -> Result<MaintenanceActionResult, String> {
        let adapter = self.providers.require(&request.provider)?;
        let descriptor = self
            .descriptor(&request.provider, &request.action_id)
            .ok_or_else(|| {
                format!(
                    "{} declares no maintenance action '{}'",
                    adapter.descriptor().display_name,
                    request.action_id
                )
            })?;
        if descriptor.requires_confirmation && !request.confirmed {
            return Err(format!("'{}' requires confirmation", descriptor.label));
        }
        let executor = self
            .executors
            .get(&(request.provider.clone(), request.action_id.clone()))
            .ok_or_else(|| {
                format!(
                    "this build has no executor for maintenance action '{}'",
                    request.action_id
                )
            })?;
        executor.execute(repository, request)
    }
}

pub(super) fn action(id: &str) -> ActionKey {
    ActionKey::parse(id).expect("static action key")
}

#[cfg(test)]
mod test_provider {
    use super::*;
    use mallard_infrastructure::providers::test_provider::provider_key;

    pub(super) fn registration() -> Registration {
        Registration {
            executors: vec![(
                provider_key(),
                action("testprov.home.inspect"),
                Arc::new(InspectHome),
            )],
        }
    }

    struct InspectHome;

    impl MaintenanceExecutor for InspectHome {
        fn execute(
            &self,
            _repository: &LocalStateRepository,
            request: &MaintenanceActionRequest,
        ) -> Result<MaintenanceActionResult, String> {
            Ok(MaintenanceActionResult {
                provider: request.provider.clone(),
                action_id: request.action_id.clone(),
                ready: true,
                summary: "Test provider home inspected.".to_string(),
                summary_code: None,
                app_name: None,
                home_path: None,
                facts: BoundedConfigObject::from_json(&serde_json::json!({"inspected": true}))?,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_infrastructure::providers::codex::provider_key as codex_key;
    use mallard_infrastructure::providers::test_provider::provider_key as test_provider_key;

    fn request(
        provider: ProviderKey,
        action_id: &str,
        confirmed: bool,
    ) -> MaintenanceActionRequest {
        MaintenanceActionRequest {
            provider,
            action_id: action(action_id),
            arguments: BoundedConfigObject::empty(),
            confirmed,
        }
    }

    #[test]
    fn production_declares_confirmed_codex_app_actions() {
        let registry = MaintenanceRegistry::production();
        let codex = codex_key();
        let restart = registry
            .descriptor(&codex, &action("codex.app.restart"))
            .unwrap();
        assert!(restart.requires_confirmation);
        assert!(
            registry
                .descriptor(&codex, &action("codex.app.close"))
                .unwrap()
                .requires_confirmation
        );
    }

    #[test]
    fn undeclared_and_unconfirmed_actions_are_refused_before_execution() {
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let registry = MaintenanceRegistry::production();
        let undeclared = registry
            .execute(&repository, &request(codex_key(), "codex.unknown", true))
            .unwrap_err();
        assert!(undeclared.contains("declares no maintenance action"));
        let unconfirmed = registry
            .execute(
                &repository,
                &request(codex_key(), "codex.app.restart", false),
            )
            .unwrap_err();
        assert!(unconfirmed.contains("requires confirmation"));
        assert!(registry
            .execute(&repository, &request(codex_key(), "codex.app.close", false))
            .unwrap_err()
            .contains("requires confirmation"));
    }

    #[test]
    fn a_test_provider_action_executes_through_registration_alone() {
        let registry = MaintenanceRegistry::with_test_provider();
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let result = registry
            .execute(
                &repository,
                &request(test_provider_key(), "testprov.home.inspect", false),
            )
            .unwrap();
        assert!(result.ready);
        assert_eq!(
            result.facts.to_json(),
            serde_json::json!({"inspected": true})
        );
        assert!(MaintenanceRegistry::production()
            .execute(
                &repository,
                &request(test_provider_key(), "testprov.home.inspect", false),
            )
            .is_err());
    }
}
