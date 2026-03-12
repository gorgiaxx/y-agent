//! y-diagnostics: trace storage, cost intelligence, search, replay, and
//! event-bus subscriber for capturing runtime observations.
//!
//! All storage is abstracted behind the [`TraceStore`] trait.  An in-memory
//! implementation is provided for testing; a `PostgreSQL` backend is available
//! behind the `diagnostics_pg` feature flag via [`PgTraceStore`].

pub mod cost;
pub mod pg_trace_store;
pub mod replay;
pub mod search;
pub mod subscriber;
pub mod trace_store;
pub mod types;

// Re-exports for convenient access.
pub use cost::CostIntelligence;
pub use replay::TraceReplay;
pub use search::{TraceSearch, TraceSearchQuery};
pub use subscriber::DiagnosticsSubscriber;
pub use trace_store::{InMemoryTraceStore, TraceStore, TraceStoreError};
pub use types::*;

#[cfg(feature = "diagnostics_pg")]
pub use pg_trace_store::PgTraceStore;
