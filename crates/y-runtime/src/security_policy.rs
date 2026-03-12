//! Security policy: configurable security profiles for runtime execution.
//!
//! Provides a unified `SecurityPolicy` that enforces network isolation,
//! filesystem restrictions, and Linux capability mapping using configurable
//! profiles (strict, standard, permissive).

use std::path::Path;

use y_core::runtime::{
    ExecutionRequest, FilesystemCapability, MountMode, NetworkCapability, RuntimeError,
};

use crate::config::RuntimeConfig;

// ---------------------------------------------------------------------------
// Security profiles
// ---------------------------------------------------------------------------

/// Predefined security profiles with different levels of restriction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityProfile {
    /// Strict: no network, no filesystem, no capabilities.
    Strict,
    /// Standard: limited network (whitelisted domains), limited filesystem
    /// (whitelisted paths), minimal capabilities.
    Standard,
    /// Permissive: mostly unrestricted; for trusted tools in development.
    Permissive,
}

impl Default for SecurityProfile {
    fn default() -> Self {
        Self::Standard
    }
}

// ---------------------------------------------------------------------------
// SecurityPolicy
// ---------------------------------------------------------------------------

/// Enforces security restrictions based on the active profile and config.
///
/// The security policy is a dedicated enforcement layer that sits between
/// capability checking and runtime dispatch:
/// 1. `CapabilityChecker` validates the request declares valid capabilities.
/// 2. `SecurityPolicy` enforces that the declared capabilities are permitted
///    under the current security profile.
/// 3. The runtime backend applies OS-level isolation.
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    profile: SecurityProfile,
    allowed_domains: std::collections::HashSet<String>,
    allowed_mounts: std::collections::HashSet<String>,
    allowed_paths: Vec<String>,
    allow_shell: bool,
}

impl SecurityPolicy {
    /// Create a security policy from the runtime configuration.
    pub fn from_config(config: &RuntimeConfig) -> Self {
        Self {
            profile: SecurityProfile::Standard,
            allowed_domains: config.allowed_domains.clone(),
            allowed_mounts: config.allowed_mounts.clone(),
            allowed_paths: config.allowed_paths.clone(),
            allow_shell: config.allow_shell,
        }
    }

    /// Create a security policy with the given profile, deriving limits from
    /// the config.
    pub fn with_profile(config: &RuntimeConfig, profile: SecurityProfile) -> Self {
        let mut policy = Self::from_config(config);
        policy.profile = profile;
        policy
    }

    /// Get the active profile.
    pub fn profile(&self) -> &SecurityProfile {
        &self.profile
    }

    /// Enforce the security policy on an execution request.
    ///
    /// Returns `Ok(())` if the request is allowed, or a `RuntimeError`
    /// describing the denial.
    pub fn enforce(&self, request: &ExecutionRequest) -> Result<(), RuntimeError> {
        self.check_network(&request.capabilities.network)?;
        self.check_filesystem(&request.capabilities.filesystem)?;
        self.check_process(request)?;
        self.check_working_dir(request)?;
        Ok(())
    }

    /// Enforce network policy.
    fn check_network(&self, cap: &NetworkCapability) -> Result<(), RuntimeError> {
        match &self.profile {
            SecurityProfile::Strict => {
                // Strict: no network access at all.
                if !matches!(cap, NetworkCapability::None) {
                    return Err(RuntimeError::CapabilityDenied {
                        capability: "network: strict profile denies all network access".into(),
                    });
                }
            }
            SecurityProfile::Standard => {
                match cap {
                    NetworkCapability::None => {}
                    NetworkCapability::Full => {
                        return Err(RuntimeError::CapabilityDenied {
                            capability:
                                "network: standard profile denies full access; use whitelisted domains"
                                    .into(),
                        });
                    }
                    NetworkCapability::External { domains } => {
                        // Verify all requested domains are whitelisted.
                        for domain in domains {
                            if !self.allowed_domains.contains(domain) {
                                return Err(RuntimeError::CapabilityDenied {
                                    capability: format!(
                                        "network: domain '{domain}' not in allowed list"
                                    ),
                                });
                            }
                        }
                    }
                    NetworkCapability::Internal { .. } => {
                        // Internal CIDRs allowed in standard mode.
                    }
                }
            }
            SecurityProfile::Permissive => {
                // Permissive: allow everything.
            }
        }
        Ok(())
    }

    /// Enforce filesystem policy.
    fn check_filesystem(&self, cap: &FilesystemCapability) -> Result<(), RuntimeError> {
        match &self.profile {
            SecurityProfile::Strict => {
                if !cap.mounts.is_empty() {
                    return Err(RuntimeError::CapabilityDenied {
                        capability: "filesystem: strict profile denies all mounts".into(),
                    });
                }
            }
            SecurityProfile::Standard => {
                for mount in &cap.mounts {
                    if !self.is_mount_allowed(&mount.host_path) {
                        return Err(RuntimeError::CapabilityDenied {
                            capability: format!(
                                "filesystem: mount '{}' not in allowed list",
                                mount.host_path
                            ),
                        });
                    }
                    if mount.mode != MountMode::ReadOnly {
                        tracing::warn!(
                            path = %mount.host_path,
                            "standard profile allows write mount; consider read-only"
                        );
                    }
                }
            }
            SecurityProfile::Permissive => {}
        }
        Ok(())
    }

    /// Check if a mount path is in the allowed list.
    fn is_mount_allowed(&self, host_path: &str) -> bool {
        if self.allowed_mounts.is_empty() {
            return false;
        }
        let path = Path::new(host_path);
        for allowed in &self.allowed_mounts {
            if path.starts_with(Path::new(allowed)) {
                return true;
            }
        }
        false
    }

    /// Enforce process capability policy.
    fn check_process(&self, request: &ExecutionRequest) -> Result<(), RuntimeError> {
        if request.capabilities.process.shell && !self.allow_shell {
            return Err(RuntimeError::CapabilityDenied {
                capability: "process.shell: shell execution denied by security policy".into(),
            });
        }
        Ok(())
    }

    /// Enforce working directory policy.
    fn check_working_dir(&self, request: &ExecutionRequest) -> Result<(), RuntimeError> {
        if self.profile == SecurityProfile::Permissive || self.allowed_paths.is_empty() {
            return Ok(());
        }

        if let Some(ref dir) = request.working_dir {
            let canonical = match std::fs::canonicalize(dir) {
                Ok(p) => p,
                Err(_) if self.profile == SecurityProfile::Strict => {
                    return Err(RuntimeError::PathTraversalAttempt {
                        path: dir.clone(),
                    });
                }
                Err(_) => return Ok(()),
            };

            for allowed in &self.allowed_paths {
                let allowed_canonical = std::fs::canonicalize(allowed)
                    .unwrap_or_else(|_| Path::new(allowed).to_path_buf());
                if canonical.starts_with(&allowed_canonical) {
                    return Ok(());
                }
            }

            return Err(RuntimeError::PathTraversalAttempt {
                path: dir.clone(),
            });
        }

        Ok(())
    }

    /// Map requested Linux capabilities to what the security profile allows.
    ///
    /// Returns the list of capabilities to **add back** after dropping all.
    pub fn allowed_linux_capabilities(&self) -> Vec<String> {
        match &self.profile {
            SecurityProfile::Strict => vec![],
            SecurityProfile::Standard => vec!["CAP_DAC_OVERRIDE".into()],
            SecurityProfile::Permissive => vec![
                "CAP_DAC_OVERRIDE".into(),
                "CAP_FOWNER".into(),
                "CAP_NET_BIND_SERVICE".into(),
                "CAP_SETGID".into(),
                "CAP_SETUID".into(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use y_core::runtime::{MountSpec, RuntimeCapability};

    use super::*;

    fn default_config() -> RuntimeConfig {
        RuntimeConfig::default()
    }

    fn make_request(net: NetworkCapability) -> ExecutionRequest {
        ExecutionRequest {
            command: "test".into(),
            args: vec![],
            working_dir: None,
            env: HashMap::new(),
            stdin: None,
            capabilities: RuntimeCapability {
                network: net,
                ..Default::default()
            },
            image: None,
        }
    }

    // T-R3-01: SecurityPolicy denies network when not allowed.
    #[test]
    fn test_strict_denies_network() {
        let config = default_config();
        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Strict);

        let req = make_request(NetworkCapability::Full);
        assert!(matches!(
            policy.enforce(&req),
            Err(RuntimeError::CapabilityDenied { .. })
        ));

        let req = make_request(NetworkCapability::External {
            domains: vec!["example.com".into()],
        });
        assert!(matches!(
            policy.enforce(&req),
            Err(RuntimeError::CapabilityDenied { .. })
        ));
    }

    #[test]
    fn test_strict_allows_no_network() {
        let config = default_config();
        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Strict);
        let req = make_request(NetworkCapability::None);
        assert!(policy.enforce(&req).is_ok());
    }

    #[test]
    fn test_standard_denies_full_network() {
        let config = default_config();
        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Standard);
        let req = make_request(NetworkCapability::Full);
        assert!(matches!(
            policy.enforce(&req),
            Err(RuntimeError::CapabilityDenied { .. })
        ));
    }

    #[test]
    fn test_standard_allows_whitelisted_domains() {
        let mut config = default_config();
        config
            .allowed_domains
            .insert("api.example.com".to_string());

        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Standard);
        let req = make_request(NetworkCapability::External {
            domains: vec!["api.example.com".into()],
        });
        assert!(policy.enforce(&req).is_ok());
    }

    #[test]
    fn test_standard_denies_non_whitelisted_domain() {
        let mut config = default_config();
        config
            .allowed_domains
            .insert("api.example.com".to_string());

        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Standard);
        let req = make_request(NetworkCapability::External {
            domains: vec!["evil.com".into()],
        });
        assert!(matches!(
            policy.enforce(&req),
            Err(RuntimeError::CapabilityDenied { .. })
        ));
    }

    #[test]
    fn test_permissive_allows_all_network() {
        let config = default_config();
        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Permissive);
        let req = make_request(NetworkCapability::Full);
        assert!(policy.enforce(&req).is_ok());
    }

    // T-R3-02: SecurityPolicy allows declared filesystem paths only.
    #[test]
    fn test_strict_denies_all_mounts() {
        let config = default_config();
        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Strict);

        let mut req = make_request(NetworkCapability::None);
        req.capabilities.filesystem.mounts = vec![MountSpec {
            host_path: "/tmp".into(),
            container_path: "/data".into(),
            mode: MountMode::ReadOnly,
        }];

        assert!(matches!(
            policy.enforce(&req),
            Err(RuntimeError::CapabilityDenied { .. })
        ));
    }

    #[test]
    fn test_standard_allows_whitelisted_mount() {
        let mut config = default_config();
        config.allowed_mounts.insert("/tmp/workspace".to_string());

        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Standard);

        let mut req = make_request(NetworkCapability::None);
        req.capabilities.filesystem.mounts = vec![MountSpec {
            host_path: "/tmp/workspace/project".into(),
            container_path: "/work".into(),
            mode: MountMode::ReadOnly,
        }];

        assert!(policy.enforce(&req).is_ok());
    }

    #[test]
    fn test_standard_denies_non_whitelisted_mount() {
        let mut config = default_config();
        config.allowed_mounts.insert("/tmp/workspace".to_string());

        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Standard);

        let mut req = make_request(NetworkCapability::None);
        req.capabilities.filesystem.mounts = vec![MountSpec {
            host_path: "/etc/passwd".into(),
            container_path: "/data".into(),
            mode: MountMode::ReadOnly,
        }];

        assert!(matches!(
            policy.enforce(&req),
            Err(RuntimeError::CapabilityDenied { .. })
        ));
    }

    #[test]
    fn test_linux_capabilities_strict() {
        let config = default_config();
        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Strict);
        assert!(policy.allowed_linux_capabilities().is_empty());
    }

    #[test]
    fn test_linux_capabilities_standard() {
        let config = default_config();
        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Standard);
        let caps = policy.allowed_linux_capabilities();
        assert_eq!(caps.len(), 1);
        assert!(caps.contains(&"CAP_DAC_OVERRIDE".to_string()));
    }

    #[test]
    fn test_linux_capabilities_permissive() {
        let config = default_config();
        let policy = SecurityPolicy::with_profile(&config, SecurityProfile::Permissive);
        let caps = policy.allowed_linux_capabilities();
        assert!(caps.len() >= 3);
    }
}
