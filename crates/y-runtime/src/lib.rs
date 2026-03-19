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
//!
//! # Observability
//!
//! - [`AuditTrail`]: Structured, append-only log of all runtime operations
//! - [`ResourceMonitor`]: Tracks CPU, memory, disk, tasks with threshold alerts

pub mod audit;
pub mod capability;
pub mod cleanup;
pub mod config;
pub mod docker;
pub mod error;
pub mod image_whitelist;
pub mod integration;
pub mod manager;
pub mod native;
pub mod resource_monitor;
pub mod security_policy;
pub mod ssh;
pub mod venv;

// Re-export primary types.
pub use audit::{AuditEntry, AuditEventType, AuditOutcome, AuditTrail};
pub use capability::CapabilityChecker;
pub use config::{BunVenvConfig, PythonVenvConfig, RuntimeConfig};
pub use docker::DockerRuntime;
pub use error::RuntimeModuleError;
pub use image_whitelist::{ImageWhitelist, WhitelistEntry};
pub use manager::RuntimeManager;
pub use native::NativeRuntime;
pub use resource_monitor::{
    ResourceMonitor, ResourceSnapshot, ResourceThresholds, ResourceViolation,
};
pub use security_policy::{SecurityPolicy, SecurityProfile};
pub use ssh::SshRuntime;
pub use venv::{VenvInitReport, VenvManager, VenvStatus};
