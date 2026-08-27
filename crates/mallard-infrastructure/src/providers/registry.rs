//! The compile-time provider registry.
//!
//! Production registers the Codex and Claude adapters. Tests may construct a
//! registry with an additional minimal provider to prove the common layers
//! are registration-driven. This is not a plugin system: no third-party
//! code, no dynamic ABI, no manifest-supplied commands.

use super::traits::{AgentProviderAdapter, ProviderDescriptor, SessionAdapter};
use mallard_domain::ProviderKey;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
pub struct ProviderRegistry {
    adapters: BTreeMap<ProviderKey, Arc<dyn AgentProviderAdapter>>,
}

impl ProviderRegistry {
    pub fn from_adapters(adapters: Vec<Arc<dyn AgentProviderAdapter>>) -> Self {
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

    /// A registry that additionally exposes the minimal test provider. Used
    /// by suites that prove common flows require no provider branches.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_test_provider() -> ProviderRegistry {
        ProviderRegistry::from_adapters(vec![
            Arc::new(super::codex::CodexAdapter::new()),
            Arc::new(super::claude::ClaudeAdapter::new()),
            Arc::new(super::test_provider::TestProviderAdapter::new()),
        ])
    }

    pub fn get(&self, key: &ProviderKey) -> Option<&Arc<dyn AgentProviderAdapter>> {
        self.adapters.get(key)
    }

    pub fn require(&self, key: &ProviderKey) -> Result<&Arc<dyn AgentProviderAdapter>, String> {
        self.adapters
            .get(key)
            .ok_or_else(|| format!("no provider adapter is registered for '{}'", key))
    }

    pub fn sessions(&self, key: &ProviderKey) -> Option<&dyn SessionAdapter> {
        self.adapters.get(key).and_then(|adapter| {
            // SAFETY of lifetimes: the returned trait object borrows from the
            // Arc stored in this registry, which outlives the call.
            let adapter: &dyn AgentProviderAdapter = adapter.as_ref();
            adapter.sessions()
        })
    }

    pub fn require_sessions(&self, key: &ProviderKey) -> Result<&dyn SessionAdapter, String> {
        self.sessions(key)
            .ok_or_else(|| format!("provider '{}' has no session adapter", key))
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
