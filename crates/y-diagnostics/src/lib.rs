//! y-diagnostics: trace storage, cost intelligence, search, replay, and
//! event-bus subscriber for capturing runtime observations.
//!
//! All storage is abstracted behind the [`TraceStore`] trait.  An in-memory
//! implementation is provided for testing; a SQLite-backed implementation
//! (`SqliteTraceStore`) is available via the `y-storage` crate for production
//! use.

pub mod context;
pub mod cost;
pub mod events;
#[cfg(feature = "langfuse")]
pub mod langfuse;
pub mod replay;
pub mod search;
pub mod sqlite_trace_store;
pub mod subscriber;
pub mod trace_store;
pub mod types;

// Re-exports for convenient access.
pub use context::{DiagnosticsContext, DIAGNOSTICS_CTX};
pub use cost::CostIntelligence;
pub use events::DiagnosticsEvent;
pub use replay::TraceReplay;
pub use search::{TraceSearch, TraceSearchQuery};
pub use sqlite_trace_store::SqliteTraceStore;
pub use subscriber::{
    DiagnosticsSubscriber, GenerationCompleteParams, GenerationParams, GenerationStartParams,
    SubagentCompleteParams, SubagentStartParams, TraceCompletedSummary,
};
pub use trace_store::{InMemoryTraceStore, TraceStore, TraceStoreError};
pub use types::*;
