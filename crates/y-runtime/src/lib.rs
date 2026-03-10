//! y-runtime: Isolated execution environments for tool execution.
//!
//! This crate provides three runtime backends implementing the
//! [`y_core::runtime::RuntimeAdapter`] trait:
//!
//! - [`NativeRuntime`]: Direct process execution (trusted tools)
//! - [`DockerRuntime`]: Container-based isolation (untrusted code)
//! - [`SshRuntime`]: Remote execution (deferred to Phase 5)
//!
//! The [`RuntimeManager`] selects the appropriate backend based on the
//! request's capability requirements and configured security policy.
//! [`CapabilityChecker`] enforces the 4 capability types: network,
//! filesystem, container, and process.
//!
//! # Security Model
//!
//! Tools declare their capability requirements; the runtime enforces them.
//! Tools never handle their own security — that is always the runtime's job.

pub mod capability;
pub mod cleanup;
pub mod config;
pub mod docker;
pub mod error;
pub mod manager;
pub mod native;
pub mod ssh;

// Re-export primary types.
pub use capability::CapabilityChecker;
pub use config::RuntimeConfig;
pub use docker::DockerRuntime;
pub use error::RuntimeModuleError;
pub use manager::RuntimeManager;
pub use native::NativeRuntime;
pub use ssh::SshRuntime;
