//! The compile-time portable resource registry for Project Sync.
//!
//! Four independent maps — resource-kind adapters, apply-policy adapters,
//! provenance codecs, and tombstone-target codecs — resolve exact versioned
//! coordinates to the closed primitives the common engine executes. Common
//! code dispatches through [`PortableResourceRegistry`] as a
//! [`PortableContractResolver`]; it never matches on a contract key.

pub mod contracts;
pub mod registry;
#[cfg(any(test, feature = "test-support"))]
pub mod test_contracts;
pub mod traits;

pub use registry::PortableResourceRegistry;
pub use traits::{
    ApplyPolicyAdapter, ProvenanceCodec, ResourceCategory, ResourceKindAdapter,
    ResourceKindDescriptor, TombstoneTargetCodec,
};
