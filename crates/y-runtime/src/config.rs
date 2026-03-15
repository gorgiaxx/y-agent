//! Runtime configuration.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use y_core::runtime::RuntimeBackend;

// ---------------------------------------------------------------------------
// SSH configuration
// ---------------------------------------------------------------------------

/// SSH authentication method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthMethod {
    /// Password-based authentication.
    Password,
    /// Public-key authentication.
    PublicKey,
}

impl Default for SshAuthMethod {
    fn default() -> Self {
        Self::PublicKey
    }
}

/// SSH connection and authentication configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    /// Remote host to connect to.
    #[serde(default = "default_ssh_host")]
    pub host: String,

    /// SSH port.
    #[serde(default = "default_ssh_port")]
    pub port: u16,

    /// Remote user.
    #[serde(default = "default_ssh_user")]
    pub user: String,

    /// Authentication method.
    #[serde(default)]
    pub auth_method: SshAuthMethod,

    /// Password (used when `auth_method = "password"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,

    /// Path to the private key file (used when `auth_method = "public_key"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_path: Option<String>,

    /// Passphrase for encrypted private keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,

    /// Path to `known_hosts` file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_hosts_path: Option<String>,
}

fn default_ssh_host() -> String {
    "localhost".into()
}

fn default_ssh_port() -> u16 {
    22
}

fn default_ssh_user() -> String {
    "root".into()
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            host: default_ssh_host(),
            port: default_ssh_port(),
            user: default_ssh_user(),
            auth_method: SshAuthMethod::default(),
            password: None,
            private_key_path: None,
            passphrase: None,
            known_hosts_path: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Docker configuration
// ---------------------------------------------------------------------------

/// A default volume mapping for Docker containers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMapping {
    /// Path on the host.
    pub host_path: String,
    /// Path inside the container.
    pub container_path: String,
    /// Access mode: `"ro"` or `"rw"`.
    #[serde(default = "default_volume_mode")]
    pub mode: String,
}

fn default_volume_mode() -> String {
    "ro".into()
}

/// Docker-specific execution defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerConfig {
    /// Default container image used when no image is specified per-request.
    ///
    /// When set, `CommandRunner::run_command` will use this image so that
    /// Docker-backend executions work without an explicit `image` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_image: Option<String>,

    /// Default network mode: `"none"`, `"bridge"`, `"host"`, or a custom name.
    #[serde(default = "default_docker_network")]
    pub network_mode: String,

    /// Run containers in privileged mode.
    #[serde(default)]
    pub privileged: bool,

    /// Container user (e.g. `"1000:1000"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    /// Whether the root filesystem is read-only.
    #[serde(default = "default_true")]
    pub readonly_rootfs: bool,

    /// Default environment variables injected into every container.
    #[serde(default)]
    pub default_env: HashMap<String, String>,

    /// Default volume mappings.
    #[serde(default)]
    pub default_volumes: Vec<VolumeMapping>,

    /// Extra `/etc/hosts` entries (e.g. `"myhost:192.168.1.1"`).
    #[serde(default)]
    pub extra_hosts: Vec<String>,

    /// Custom DNS servers.
    #[serde(default)]
    pub dns: Vec<String>,

    /// Linux capabilities to add (default empty).
    #[serde(default)]
    pub cap_add: Vec<String>,

    /// Linux capabilities to drop (default `["ALL"]`).
    #[serde(default = "default_cap_drop")]
    pub cap_drop: Vec<String>,
}

fn default_docker_network() -> String {
    "none".into()
}

fn default_true() -> bool {
    true
}

fn default_cap_drop() -> Vec<String> {
    vec!["ALL".into()]
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            default_image: None,
            network_mode: default_docker_network(),
            privileged: false,
            user: None,
            readonly_rootfs: true,
            default_env: HashMap::new(),
            default_volumes: vec![],
            extra_hosts: vec![],
            dns: vec![],
            cap_add: vec![],
            cap_drop: default_cap_drop(),
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level runtime configuration
// ---------------------------------------------------------------------------

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

    /// SSH backend configuration.
    #[serde(default)]
    pub ssh: SshConfig,

    /// Docker backend configuration.
    #[serde(default)]
    pub docker: DockerConfig,
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
            ssh: SshConfig::default(),
            docker: DockerConfig::default(),
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

    #[test]
    fn test_ssh_config_default() {
        let ssh = SshConfig::default();
        assert_eq!(ssh.host, "localhost");
        assert_eq!(ssh.port, 22);
        assert_eq!(ssh.user, "root");
        assert_eq!(ssh.auth_method, SshAuthMethod::PublicKey);
        assert!(ssh.password.is_none());
        assert!(ssh.private_key_path.is_none());
        assert!(ssh.passphrase.is_none());
        assert!(ssh.known_hosts_path.is_none());
    }

    #[test]
    fn test_docker_config_default() {
        let docker = DockerConfig::default();
        assert!(docker.default_image.is_none());
        assert_eq!(docker.network_mode, "none");
        assert!(!docker.privileged);
        assert!(docker.user.is_none());
        assert!(docker.readonly_rootfs);
        assert!(docker.default_env.is_empty());
        assert!(docker.default_volumes.is_empty());
        assert!(docker.extra_hosts.is_empty());
        assert!(docker.dns.is_empty());
        assert!(docker.cap_add.is_empty());
        assert_eq!(docker.cap_drop, vec!["ALL".to_string()]);
    }

    #[test]
    fn test_ssh_config_toml_roundtrip() {
        let toml_str = r#"
[ssh]
host = "10.0.0.5"
port = 2222
user = "deploy"
auth_method = "password"
password = "secret123"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.ssh.host, "10.0.0.5");
        assert_eq!(config.ssh.port, 2222);
        assert_eq!(config.ssh.user, "deploy");
        assert_eq!(config.ssh.auth_method, SshAuthMethod::Password);
        assert_eq!(config.ssh.password.as_deref(), Some("secret123"));
        assert!(config.ssh.private_key_path.is_none());
    }

    #[test]
    fn test_ssh_config_pubkey_toml() {
        let toml_str = r#"
[ssh]
host = "prod.example.com"
auth_method = "public_key"
private_key_path = "/home/user/.ssh/id_ed25519"
passphrase = "mypass"
known_hosts_path = "/home/user/.ssh/known_hosts"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.ssh.auth_method, SshAuthMethod::PublicKey);
        assert_eq!(
            config.ssh.private_key_path.as_deref(),
            Some("/home/user/.ssh/id_ed25519")
        );
        assert_eq!(config.ssh.passphrase.as_deref(), Some("mypass"));
        assert_eq!(
            config.ssh.known_hosts_path.as_deref(),
            Some("/home/user/.ssh/known_hosts")
        );
    }

    #[test]
    fn test_docker_config_toml_roundtrip() {
        let toml_str = r#"
[docker]
network_mode = "bridge"
privileged = true
user = "1000:1000"
readonly_rootfs = false
dns = ["8.8.8.8", "8.8.4.4"]
cap_add = ["NET_ADMIN"]
cap_drop = ["ALL"]
extra_hosts = ["myhost:192.168.1.1"]

[docker.default_env]
RUST_LOG = "debug"
TZ = "UTC"

[[docker.default_volumes]]
host_path = "/data/shared"
container_path = "/mnt/data"
mode = "rw"

[[docker.default_volumes]]
host_path = "/etc/ssl/certs"
container_path = "/etc/ssl/certs"
mode = "ro"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.docker.network_mode, "bridge");
        assert!(config.docker.privileged);
        assert_eq!(config.docker.user.as_deref(), Some("1000:1000"));
        assert!(!config.docker.readonly_rootfs);
        assert_eq!(config.docker.dns, vec!["8.8.8.8", "8.8.4.4"]);
        assert_eq!(config.docker.cap_add, vec!["NET_ADMIN"]);
        assert_eq!(config.docker.cap_drop, vec!["ALL"]);
        assert_eq!(config.docker.extra_hosts, vec!["myhost:192.168.1.1"]);
        assert_eq!(config.docker.default_env.get("RUST_LOG").unwrap(), "debug");
        assert_eq!(config.docker.default_env.get("TZ").unwrap(), "UTC");
        assert_eq!(config.docker.default_volumes.len(), 2);
        assert_eq!(config.docker.default_volumes[0].host_path, "/data/shared");
        assert_eq!(config.docker.default_volumes[0].mode, "rw");
        assert_eq!(config.docker.default_volumes[1].mode, "ro");
    }

    #[test]
    fn test_full_config_with_ssh_and_docker() {
        let toml_str = r#"
default_backend = "docker"
allow_shell = true
default_timeout = "1m"

[ssh]
host = "remote.server.io"
port = 22
user = "admin"
auth_method = "public_key"
private_key_path = "~/.ssh/id_rsa"

[docker]
network_mode = "bridge"
privileged = false
readonly_rootfs = true
cap_drop = ["ALL"]

[[docker.default_volumes]]
host_path = "/workspace"
container_path = "/app"
mode = "rw"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.default_backend, RuntimeBackend::Docker);
        assert!(config.allow_shell);
        assert_eq!(config.default_timeout, Duration::from_secs(60));
        assert_eq!(config.ssh.host, "remote.server.io");
        assert_eq!(config.ssh.auth_method, SshAuthMethod::PublicKey);
        assert_eq!(config.docker.network_mode, "bridge");
        assert!(!config.docker.privileged);
        assert_eq!(config.docker.default_volumes.len(), 1);
        assert_eq!(config.docker.default_volumes[0].host_path, "/workspace");
    }
}
