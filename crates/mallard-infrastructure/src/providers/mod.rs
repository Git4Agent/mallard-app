//! Compile-time agent-provider registry and adapters.
//!
//! Common project-sync code discovers providers exclusively through
//! [`registry::ProviderRegistry`] descriptors and the typed adapter traits in
//! [`traits`]; it never imports a concrete adapter or branches on a provider
//! key. Adding a built-in provider is an adapter module plus one registry
//! entry — no changes to setup, Activity, reconciliation, review, or storage
//! code.

pub mod claude;
pub mod codex;
pub mod launch;
pub mod registry;
#[cfg(any(test, feature = "test-support"))]
pub mod test_provider;
pub mod timeparse;
pub mod traits;

pub use registry::ProviderRegistry;
pub use traits::{
    AgentProviderAdapter, DiscoveredProviderState, DiscoveredSession, DiscoveredSidecar,
    DiscoveredStateFile, MaterializationTargetRequest, PlannedProviderTarget,
    ProviderCapabilityLevel, ProviderDescriptor, SessionActionContext, SessionActionDescriptor,
    SessionActionKind, SessionActionOutcome, SessionActionRequest, SessionAdapter, SessionAnalysis,
    SessionCheckpoint, SessionCount, SessionCountQuality, SessionDetailWarning, SessionDiscovery,
    SessionDiscoveryContext, SessionEventCounts, SessionEventDetail, SessionFileMetadata,
    SessionFileSummary, SessionRewriteRequest, SessionSourceSpan, SessionTimelineContext,
    SessionTimelineEvent, SessionTimelineEventKind, SessionTimelineIndex,
    SessionTimelineIndexEntry, SessionTimelinePageSlice, SessionTimelineQuery,
    SessionToolCallPreview, SessionToolCallStatus, SessionToolCategory, SessionToolIndexEntry,
    SessionTurnToolSummary,
};
