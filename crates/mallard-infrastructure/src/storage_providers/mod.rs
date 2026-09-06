//! Compile-time storage-provider registry and adapters.
//!
//! Common storage code resolves a configured storage's provider key and
//! config version through [`registry::StorageProviderRegistry`] and then
//! speaks only the [`traits::StorageProviderAdapter`] contract. It never
//! imports a concrete adapter or branches on a provider key. Adding a
//! built-in storage provider is an adapter module plus one registry entry.

pub mod local;
pub mod r2;
pub mod registry;
#[cfg(any(test, feature = "test-support"))]
pub mod test_provider;
pub mod traits;

pub use registry::StorageProviderRegistry;
pub use traits::{
    StorageAdapterError, StorageAdapterErrorKind, StorageFieldDescriptor, StorageFieldKind,
    StorageProviderAdapter, StorageProviderDescriptor, StorageSecretDescriptor,
    StorageSetupAssistantContext, StorageSetupAssistantDescriptor, StorageSummaryField,
};
