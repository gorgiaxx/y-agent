//! y-test-utils: Shared test utilities, mocks, and fixtures for y-agent.
//!
//! This crate provides:
//! - Mock implementations of y-core traits (providers, runtime, storage)
//! - Factory functions for creating test data
//! - Custom assertion helpers

pub mod assert_helpers;
pub mod fixtures;
pub mod mock_provider;
pub mod mock_runtime;
pub mod mock_storage;

// Re-exports for convenient access.
pub use fixtures::*;
pub use mock_provider::{MockBehaviour, MockProvider};
pub use mock_runtime::MockRuntime;
pub use mock_storage::{MockCheckpointStorage, MockSessionStore, MockTranscriptStore};
