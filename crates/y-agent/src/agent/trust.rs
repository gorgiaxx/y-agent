//! Trust tiers for permission scoping across agents.
//!
//! The canonical definition lives in `y_core::trust`. This module
//! re-exports the type for backward compatibility within the `y-agent` crate.

pub use y_core::trust::TrustTier;
