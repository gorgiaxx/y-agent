//! Crate-level error types for y-storage.

use y_core::error::{ClassifiedError, ErrorSeverity};

/// Errors from storage operations.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {message}")]
    Database { message: String },

    #[error("migration error: {message}")]
    Migration { message: String },

    #[error("not found: {entity} with id {id}")]
    NotFound { entity: String, id: String },

    #[error("invalid configuration: {message}")]
    Config { message: String },

    #[error("serialization error: {message}")]
    Serialization { message: String },

    #[error("transcript I/O error: {message}")]
    TranscriptIo { message: String },

    #[error("stale checkpoint: expected version {expected}, found {found}")]
    StaleCheckpoint { expected: u64, found: u64 },

    #[error("pool exhausted: {message}")]
    PoolExhausted { message: String },

    #[error("connection error: {message}")]
    Connection { message: String },

    #[error("{message}")]
    Other { message: String },
}

impl ClassifiedError for StorageError {
    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Database { .. } | Self::PoolExhausted { .. } | Self::Connection { .. }
        )
    }

    fn error_code(&self) -> &str {
        match self {
            Self::Database { .. } => "STORAGE_DATABASE_ERROR",
            Self::Migration { .. } => "STORAGE_MIGRATION_ERROR",
            Self::NotFound { .. } => "STORAGE_NOT_FOUND",
            Self::Config { .. } => "STORAGE_CONFIG_ERROR",
            Self::Serialization { .. } => "STORAGE_SERIALIZATION_ERROR",
            Self::TranscriptIo { .. } => "STORAGE_TRANSCRIPT_IO",
            Self::StaleCheckpoint { .. } => "STORAGE_STALE_CHECKPOINT",
            Self::PoolExhausted { .. } => "STORAGE_POOL_EXHAUSTED",
            Self::Connection { .. } => "STORAGE_CONNECTION_ERROR",
            Self::Other { .. } => "STORAGE_OTHER",
        }
    }

    fn severity(&self) -> ErrorSeverity {
        match self {
            Self::Database { .. } | Self::PoolExhausted { .. } | Self::Connection { .. } => {
                ErrorSeverity::Transient
            }
            Self::Config { .. } => ErrorSeverity::UserActionRequired,
            _ => ErrorSeverity::Permanent,
        }
    }
}

impl From<sqlx::Error> for StorageError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database {
            message: err.to_string(),
        }
    }
}

impl From<sqlx::migrate::MigrateError> for StorageError {
    fn from(err: sqlx::migrate::MigrateError) -> Self {
        Self::Migration {
            message: err.to_string(),
        }
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization {
            message: err.to_string(),
        }
    }
}

impl From<std::io::Error> for StorageError {
    fn from(err: std::io::Error) -> Self {
        Self::TranscriptIo {
            message: err.to_string(),
        }
    }
}
