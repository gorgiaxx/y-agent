//! Storage configuration.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::StorageError;

/// Configuration for the storage layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Path to the `SQLite` database file. Use `:memory:` for in-memory.
    #[serde(default = "default_db_path")]
    pub db_path: String,

    /// Maximum number of connections in the pool.
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,

    /// Enable WAL mode (strongly recommended for concurrency).
    #[serde(default = "default_wal_enabled")]
    pub wal_enabled: bool,

    /// Busy timeout in milliseconds.
    #[serde(default = "default_busy_timeout_ms")]
    pub busy_timeout_ms: u32,

    /// Directory where JSONL transcript files are stored.
    #[serde(default = "default_transcript_dir")]
    pub transcript_dir: PathBuf,

    /// Path to the migrations directory.
    #[serde(default = "default_migrations_dir")]
    pub migrations_dir: PathBuf,
}

fn default_db_path() -> String {
    "data/y-agent.db".to_string()
}

fn default_pool_size() -> u32 {
    5
}

fn default_wal_enabled() -> bool {
    true
}

fn default_busy_timeout_ms() -> u32 {
    5000
}

fn default_transcript_dir() -> PathBuf {
    PathBuf::from("data/transcripts")
}

fn default_migrations_dir() -> PathBuf {
    PathBuf::from("migrations/sqlite")
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            pool_size: default_pool_size(),
            wal_enabled: default_wal_enabled(),
            busy_timeout_ms: default_busy_timeout_ms(),
            transcript_dir: default_transcript_dir(),
            migrations_dir: default_migrations_dir(),
        }
    }
}

impl StorageConfig {
    /// Validate the configuration, returning an error if invalid.
    pub fn validate(&self) -> Result<(), StorageError> {
        if self.db_path.is_empty() {
            return Err(StorageError::Config {
                message: "db_path must not be empty".into(),
            });
        }

        if self.pool_size == 0 {
            return Err(StorageError::Config {
                message: "pool_size must be greater than 0".into(),
            });
        }

        if self.transcript_dir.as_os_str().is_empty() {
            return Err(StorageError::Config {
                message: "transcript_dir must not be empty".into(),
            });
        }

        Ok(())
    }

    /// Create a config for in-memory database (useful for testing).
    pub fn in_memory() -> Self {
        Self {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false, // WAL not meaningful for :memory:
            ..Self::default()
        }
    }

    /// Check whether this config targets an in-memory database.
    pub fn is_in_memory(&self) -> bool {
        self.db_path == ":memory:"
    }

    /// Get the database path as a `Path` reference.
    pub fn db_dir(&self) -> Option<&Path> {
        if self.is_in_memory() {
            None
        } else {
            Path::new(&self.db_path).parent()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default_values() {
        let config = StorageConfig::default();
        assert!(config.wal_enabled, "WAL mode should be enabled by default");
        assert!(config.pool_size > 0, "pool_size should be > 0");
        assert_eq!(config.busy_timeout_ms, 5000);
        assert!(!config.db_path.is_empty());
    }

    #[test]
    fn test_config_validate_empty_path_fails() {
        let config = StorageConfig {
            db_path: String::new(),
            ..StorageConfig::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("db_path"));
    }

    #[test]
    fn test_config_validate_valid_config() {
        let config = StorageConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_deserialization_from_toml() {
        let toml_str = r#"
            db_path = "/tmp/test.db"
            pool_size = 10
            wal_enabled = true
            busy_timeout_ms = 3000
            transcript_dir = "/tmp/transcripts"
            migrations_dir = "migrations/sqlite"
        "#;
        let config: StorageConfig = toml::from_str(toml_str).expect("should parse TOML");
        assert_eq!(config.db_path, "/tmp/test.db");
        assert_eq!(config.pool_size, 10);
        assert!(config.wal_enabled);
        assert_eq!(config.busy_timeout_ms, 3000);
        assert_eq!(config.transcript_dir, PathBuf::from("/tmp/transcripts"));
    }

    #[test]
    fn test_config_in_memory() {
        let config = StorageConfig::in_memory();
        assert!(config.is_in_memory());
        assert_eq!(config.pool_size, 1);
        assert!(!config.wal_enabled);
    }

    #[test]
    fn test_config_validate_zero_pool_size() {
        let config = StorageConfig {
            pool_size: 0,
            ..StorageConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
