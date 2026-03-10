//! Tool registry errors.

/// Errors specific to the tool registry implementation.
#[derive(Debug, thiserror::Error)]
pub enum ToolRegistryError {
    #[error("tool not found: {name}")]
    NotFound { name: String },

    #[error("parameter validation failed: {message}")]
    ValidationError { message: String },

    #[error("duplicate tool name: {name}")]
    DuplicateName { name: String },

    #[error("tool activation limit exceeded: max {max} active tools")]
    ActivationLimitExceeded { max: usize },

    #[error("middleware chain error: {message}")]
    MiddlewareError { message: String },

    #[error("tool execution error: {message}")]
    ExecutionError { message: String },

    #[error("{message}")]
    Other { message: String },
}

impl From<ToolRegistryError> for y_core::tool::ToolError {
    fn from(e: ToolRegistryError) -> Self {
        match e {
            ToolRegistryError::NotFound { name } => y_core::tool::ToolError::NotFound { name },
            ToolRegistryError::ValidationError { message } => {
                y_core::tool::ToolError::ValidationError { message }
            }
            ToolRegistryError::ExecutionError { message } => {
                y_core::tool::ToolError::RuntimeError {
                    name: String::new(),
                    message,
                }
            }
            other => y_core::tool::ToolError::Other {
                message: other.to_string(),
            },
        }
    }
}
