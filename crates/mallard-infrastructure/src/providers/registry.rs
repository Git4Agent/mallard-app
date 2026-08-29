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
pub struct ProviderRegistry {
    adapters: BTreeMap<ProviderKey, Arc<dyn SessionAdapter>>,
}

impl ProviderRegistry {
    pub fn from_adapters(adapters: Vec<Arc<dyn SessionAdapter>>) -> Self {
        let adapters = adapters
            .into_iter()
            .map(|adapter| (adapter.descriptor().key, adapter))
            .collect();
        Self { adapters }
    }

    /// The process-wide production registry: Codex and Claude.
    pub fn production() -> &'static ProviderRegistry {
        static REGISTRY: OnceLock<ProviderRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            ProviderRegistry::from_adapters(vec![
                Arc::new(super::codex::CodexAdapter::new()),
                Arc::new(super::claude::ClaudeAdapter::new()),
            ])
        })
    }

    /// A registry that additionally exposes the minimal discovery and capture
    /// test provider.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_test_provider() -> ProviderRegistry {
        ProviderRegistry::from_adapters(vec![
            Arc::new(super::codex::CodexAdapter::new()),
            Arc::new(super::claude::ClaudeAdapter::new()),
            Arc::new(super::test_provider::TestProviderAdapter::new()),
        ])
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

    /// Descriptors in deterministic key order, for setup and Activity.
    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        self.adapters
            .values()
            .map(|adapter| adapter.descriptor())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_provider_keeps_option_and_result_failure_paths() {
        let registry = ProviderRegistry::with_test_provider();
        let unknown = ProviderKey::parse("unknown").unwrap();

        assert!(registry.get(&unknown).is_none());
        assert_eq!(
            registry.require(&unknown).err().as_deref(),
            Some("no provider adapter is registered for 'unknown'")
        );
    }
}
