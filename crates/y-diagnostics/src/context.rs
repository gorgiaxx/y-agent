//! Ambient diagnostics context propagated via task-local.
//!
//! Set at execution entry points (`AgentService::execute`,
//! `DiagnosticsAgentDelegator::delegate`).  Both gateways read this
//! to associate observations with the correct trace.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use uuid::Uuid;

/// Ambient diagnostics context for the current task.
///
/// Carries trace identity and iteration state so that provider and tool
/// gateways can automatically record observations without any manual
/// wiring in business logic.
#[derive(Clone, Debug)]
pub struct DiagnosticsContext {
    pub trace_id: Uuid,
    pub session_id: Option<Uuid>,
    pub agent_name: String,
    /// LLM iteration counter (1-based, atomically incremented by the provider
    /// gateway each time a new generation starts).
    pub iteration: Arc<AtomicU32>,
    /// Last generation observation ID (for `parent_id` chaining on tool calls).
    pub last_gen_id: Arc<tokio::sync::Mutex<Option<Uuid>>>,
}

impl DiagnosticsContext {
    pub fn new(trace_id: Uuid, session_id: Option<Uuid>, agent_name: String) -> Self {
        Self {
            trace_id,
            session_id,
            agent_name,
            iteration: Arc::new(AtomicU32::new(0)),
            last_gen_id: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Atomically increment and return the next 1-based iteration number.
    pub fn next_iteration(&self) -> u32 {
        self.iteration.fetch_add(1, Ordering::Relaxed) + 1
    }
}

tokio::task_local! {
    pub static DIAGNOSTICS_CTX: DiagnosticsContext;
}
