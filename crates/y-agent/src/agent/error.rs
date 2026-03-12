#[derive(Debug, thiserror::Error)]
pub enum MultiAgentError {
    #[error("agent not found: {id}")]
    NotFound { id: String },

    #[error("pool limit reached: max {max} agents")]
    PoolLimitReached { max: usize },

    #[error("delegation failed: {message}")]
    DelegationFailed { message: String },

    #[error("invalid definition: {message}")]
    InvalidDefinition { message: String },

    #[error("delegation timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    #[error("delegation depth {depth} exceeds max {max}")]
    DelegationDepthExceeded { depth: usize, max: usize },

    #[error("{message}")]
    Other { message: String },
}
