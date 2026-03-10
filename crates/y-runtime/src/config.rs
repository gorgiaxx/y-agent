//! Runtime configuration.

use std::collections::HashSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use y_core::runtime::RuntimeBackend;

/// Configuration for the runtime system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Default runtime backend.
    #[serde(default = "default_backend")]
    pub default_backend: RuntimeBackend,

    /// Whitelisted container images (empty = deny all).
    #[serde(default)]
    pub image_whitelist: HashSet<String>,

    /// Whitelisted external domains for network access.
    #[serde(default)]
    pub allowed_domains: HashSet<String>,

    /// Whitelisted filesystem mount paths.
    #[serde(default)]
    pub allowed_mounts: HashSet<String>,

    /// Whether shell execution is allowed.
    #[serde(default)]
    pub allow_shell: bool,

    /// Whether host filesystem access is allowed.
    #[serde(default)]
    pub allow_host_access: bool,

    /// Default memory limit in bytes (512 MB).
    #[serde(default = "default_memory_limit")]
    pub default_memory_bytes: u64,

    /// Default CPU quota (1.0 = 1 core).
    #[serde(default = "default_cpu_quota")]
    pub default_cpu_quota: f64,

    /// Default execution timeout.
    #[serde(
        default = "default_timeout",
        with = "humantime_serde_compat"
    )]
    pub default_timeout: Duration,

    /// Maximum output size in bytes (10 MB).
    #[serde(default = "default_max_output")]
    pub default_max_output_bytes: u64,

    /// Whether to allow pulling new container images.
    #[serde(default)]
    pub allow_image_pull: bool,
}

fn default_backend() -> RuntimeBackend {
    RuntimeBackend::Native
}

fn default_memory_limit() -> u64 {
    512 * 1024 * 1024 // 512 MB
}

fn default_cpu_quota() -> f64 {
    1.0
}

fn default_timeout() -> Duration {
    Duration::from_secs(300)
}

fn default_max_output() -> u64 {
    10 * 1024 * 1024 // 10 MB
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            default_backend: default_backend(),
            image_whitelist: HashSet::new(),
            allowed_domains: HashSet::new(),
            allowed_mounts: HashSet::new(),
            allow_shell: false,
            allow_host_access: false,
            default_memory_bytes: default_memory_limit(),
            default_cpu_quota: default_cpu_quota(),
            default_timeout: default_timeout(),
            default_max_output_bytes: default_max_output(),
            allow_image_pull: false,
        }
    }
}

/// Helper module for Duration serialization (seconds as u64).
mod humantime_serde_compat {
    use std::time::Duration;

    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RuntimeConfig::default();
        assert_eq!(config.default_backend, RuntimeBackend::Native);
        assert!(config.image_whitelist.is_empty());
        assert!(!config.allow_shell);
        assert!(!config.allow_host_access);
        assert_eq!(config.default_memory_bytes, 512 * 1024 * 1024);
        assert!((config.default_cpu_quota - 1.0).abs() < f64::EPSILON);
        assert_eq!(config.default_timeout, Duration::from_secs(300));
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let config = RuntimeConfig {
            default_backend: RuntimeBackend::Docker,
            image_whitelist: HashSet::from(["python:3.11".into()]),
            allow_shell: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: RuntimeConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.default_backend, RuntimeBackend::Docker);
        assert!(deserialized.image_whitelist.contains("python:3.11"));
        assert!(deserialized.allow_shell);
    }
}
