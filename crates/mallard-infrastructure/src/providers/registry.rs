//! The compile-time provider registry.
//!
//! Production registers the Codex and Claude adapters. Tests may construct a
//! registry with an additional minimal provider to exercise shared discovery
//! and capture. This is not a plugin system: no third-party code, no dynamic
//! ABI, no manifest-supplied commands.

use super::traits::{ProviderDescriptor, SessionAdapter};
use mallard_domain::ProviderKey;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
pub struct AgentProviderRegistry {
    adapters: BTreeMap<ProviderKey, Arc<dyn SessionAdapter>>,
}

impl AgentProviderRegistry {
    /// Build a registry, refusing a repeated provider key.
    pub fn from_adapters(adapters: Vec<Arc<dyn SessionAdapter>>) -> Result<Self, String> {
        let mut map: BTreeMap<ProviderKey, Arc<dyn SessionAdapter>> = BTreeMap::new();
        for adapter in adapters {
            let key = adapter.descriptor().key;
            if map.insert(key.clone(), adapter).is_some() {
                return Err(format!("agent provider '{}' is registered twice", key));
            }
        }
        Ok(Self { adapters: map })
    }

    /// The process-wide production registry: Codex and Claude.
    pub fn production() -> &'static AgentProviderRegistry {
        static REGISTRY: OnceLock<AgentProviderRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            AgentProviderRegistry::from_adapters(vec![
                Arc::new(super::codex::CodexAdapter::new()),
                Arc::new(super::claude::ClaudeAdapter::new()),
            ])
            .expect("production agent registry has no duplicate registrations")
        })
    }

    /// A registry that additionally exposes the minimal discovery and capture
    /// test provider.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_test_provider() -> AgentProviderRegistry {
        AgentProviderRegistry::from_adapters(vec![
            Arc::new(super::codex::CodexAdapter::new()),
            Arc::new(super::claude::ClaudeAdapter::new()),
            Arc::new(super::test_provider::TestProviderAdapter::new()),
        ])
        .expect("test agent registry has no duplicate registrations")
    }

    pub fn get(&self, key: &ProviderKey) -> Option<&dyn SessionAdapter> {
        self.adapters.get(key).map(Arc::as_ref)
    }

    pub fn require(&self, key: &ProviderKey) -> Result<&dyn SessionAdapter, String> {
        self.get(key)
            .ok_or_else(|| format!("no provider adapter is registered for '{}'", key))
    }

    pub fn contains(&self, key: &ProviderKey) -> bool {
        self.adapters.contains_key(key)
    }

    /// Descriptors in display order: rank first, then key, for setup and
    /// Activity. Iteration order of the registry itself is key order.
    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        let mut descriptors = self
            .adapters
            .values()
            .map(|adapter| adapter.descriptor())
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| {
            left.display_rank
                .cmp(&right.display_rank)
                .then_with(|| left.key.cmp(&right.key))
        });
        descriptors
    }

    /// The adapter-declared tail layout codec for one provider.
    pub fn tail_layout_codec(&self, key: &ProviderKey) -> Option<&'static str> {
        self.get(key)
            .and_then(|adapter| adapter.tail_layout_codec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_registrations_are_rejected() {
        let error = AgentProviderRegistry::from_adapters(vec![
            Arc::new(super::super::codex::CodexAdapter::new()),
            Arc::new(super::super::codex::CodexAdapter::new()),
        ])
        .err()
        .expect("duplicate registration is rejected");
        assert_eq!(error, "agent provider 'codex' is registered twice");
    }

    #[test]
    fn descriptors_follow_display_rank_then_key() {
        let keys = AgentProviderRegistry::with_test_provider()
            .descriptors()
            .into_iter()
            .map(|descriptor| (descriptor.display_rank, descriptor.key.as_str().to_string()))
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                (10, "codex".to_string()),
                (20, "claude".to_string()),
                (90, "testprov".to_string())
            ]
        );
    }

    #[test]
    fn unknown_provider_keeps_option_and_result_failure_paths() {
        let registry = AgentProviderRegistry::with_test_provider();
        let unknown = ProviderKey::parse("unknown").unwrap();

        assert!(registry.get(&unknown).is_none());
        assert_eq!(
            registry.require(&unknown).err().as_deref(),
            Some("no provider adapter is registered for 'unknown'")
        );
    }
}
