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
    #[serde(default = "default_timeout", with = "humantime_serde_compat")]
    pub default_timeout: Duration,

    /// Maximum output size in bytes (10 MB).
    #[serde(default = "default_max_output")]
    pub default_max_output_bytes: u64,

    /// Whether to allow pulling new container images.
    #[serde(default)]
    pub allow_image_pull: bool,

    /// Allowed working directory paths for native execution (empty = allow all).
    #[serde(default)]
    pub allowed_paths: Vec<String>,
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
            allowed_paths: vec![],
        }
    }
}

/// Helper module for Duration serialization.
///
/// Accepts both human-readable strings ("30s", "5m", "1h30m") and plain
/// integer seconds (30) on deserialization. Serializes as a human-readable
/// string (e.g. "30s").
mod humantime_serde_compat {
    use std::fmt;
    use std::time::Duration;

    use serde::{self, de, Deserializer, Serializer};

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Emit human-readable string for config roundtrip.
        let secs = duration.as_secs();
        if secs % 3600 == 0 && secs > 0 {
            serializer.serialize_str(&format!("{}h", secs / 3600))
        } else if secs % 60 == 0 && secs > 0 {
            serializer.serialize_str(&format!("{}m", secs / 60))
        } else {
            serializer.serialize_str(&format!("{secs}s"))
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DurationVisitor;

        impl de::Visitor<'_> for DurationVisitor {
            type Value = Duration;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a duration string (e.g. \"30s\", \"5m\", \"1h\") or integer seconds")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Duration, E> {
                Ok(Duration::from_secs(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Duration, E> {
                if v < 0 {
                    return Err(E::custom("duration cannot be negative"));
                }
                Ok(Duration::from_secs(v as u64))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Duration, E> {
                parse_duration(v).map_err(E::custom)
            }
        }

        deserializer.deserialize_any(DurationVisitor)
    }

    /// Parse a simple human-readable duration string.
    ///
    /// Supported formats: `"30s"`, `"5m"`, `"1h"`, `"1h30m"`, `"300"` (bare
    /// seconds). Each segment is a number followed by a unit suffix
    /// (`h`/`m`/`s`). A bare number without suffix is treated as seconds.
    fn parse_duration(s: &str) -> Result<Duration, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty duration string".into());
        }

        // Bare integer (no suffix) → seconds.
        if let Ok(secs) = s.parse::<u64>() {
            return Ok(Duration::from_secs(secs));
        }

        let mut total_secs: u64 = 0;
        let mut chars = s.chars().peekable();

        while chars.peek().is_some() {
            // Accumulate digits.
            let mut num_str = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    num_str.push(c);
                    chars.next();
                } else {
                    break;
                }
            }

            if num_str.is_empty() {
                return Err(format!("expected a number in duration: \"{s}\""));
            }

            let n: u64 = num_str
                .parse()
                .map_err(|_| format!("invalid number in duration: \"{num_str}\""))?;

            // Read unit suffix.
            match chars.next() {
                Some('h') => total_secs += n * 3600,
                Some('m') => total_secs += n * 60,
                Some('s') => total_secs += n,
                Some(c) => return Err(format!("unknown duration unit '{c}' in \"{s}\"")),
                None => {
                    // Trailing bare number — treat as seconds.
                    total_secs += n;
                }
            }
        }

        Ok(Duration::from_secs(total_secs))
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

    #[test]
    fn test_toml_duration_string() {
        let toml_str = r#"
default_timeout = "30s"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.default_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_toml_duration_integer() {
        let toml_str = r"
default_timeout = 60
";
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.default_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_toml_duration_minutes() {
        let toml_str = r#"
default_timeout = "5m"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.default_timeout, Duration::from_secs(300));
    }

    #[test]
    fn test_toml_duration_compound() {
        let toml_str = r#"
default_timeout = "1h30m"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.default_timeout, Duration::from_secs(5400));
    }

    #[test]
    fn test_duration_parse_invalid() {
        let toml_str = r#"
default_timeout = "abc"
"#;
        assert!(toml::from_str::<RuntimeConfig>(toml_str).is_err());
    }
}
