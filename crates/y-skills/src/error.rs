//! Error types for the skills module.

/// Errors from skill module operations.
///
/// Wraps `y_core::skill::SkillError` for module-specific operations
/// like ingestion, version store I/O, and search.
#[derive(Debug, thiserror::Error)]
pub enum SkillModuleError {
    /// Forwarded from y-core `SkillError`.
    #[error(transparent)]
    Core(#[from] y_core::skill::SkillError),

    /// Version store I/O error.
    #[error("version store error: {message}")]
    VersionStoreError { message: String },

    /// Ingestion processing error.
    #[error("ingestion error: {message}")]
    IngestionError { message: String },

    /// Manifest parsing error.
    #[error("manifest parse error: {message}")]
    ManifestParseError { message: String },

    /// Serialization/deserialization error.
    #[error("serialization error: {0}")]
    SerdeError(#[from] serde_json::Error),

    /// TOML parsing error.
    #[error("TOML error: {0}")]
    TomlError(#[from] toml::de::Error),

    /// Generic error.
    #[error("{message}")]
    Other { message: String },
}
