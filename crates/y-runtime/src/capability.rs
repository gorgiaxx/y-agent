//! Capability checker: validates execution requests against configured policy.

use y_core::runtime::{
    ContainerCapability, ExecutionRequest, FilesystemCapability, NetworkCapability,
    ProcessCapability, ResourceLimits, RuntimeCapability,
};

use crate::config::RuntimeConfig;
use crate::error::RuntimeModuleError;

/// Validates capability requirements against the configured security policy.
///
/// The checker enforces the principle of least privilege: tools declare what
/// they need, the runtime checks whether the policy allows it, and caps
/// any resource limits to the configured maximums.
pub struct CapabilityChecker<'a> {
    config: &'a RuntimeConfig,
}

impl<'a> CapabilityChecker<'a> {
    /// Create a new capability checker with the given config.
    pub fn new(config: &'a RuntimeConfig) -> Self {
        Self { config }
    }

    /// Validate the full capability set of a request against policy.
    ///
    /// Returns the (potentially capped) capabilities on success,
    /// or a `CapabilityDenied` / `ImageNotAllowed` error on failure.
    pub fn validate(
        &self,
        request: &ExecutionRequest,
    ) -> Result<RuntimeCapability, RuntimeModuleError> {
        let caps = &request.capabilities;

        self.validate_network(&caps.network)?;
        self.validate_filesystem(&caps.filesystem)?;
        self.validate_container(&caps.container, request.image.as_deref())?;
        self.validate_process(&caps.process)?;

        // Return capabilities with resource limits capped to policy maximums.
        let capped = self.cap_resources(caps);
        Ok(capped)
    }

    /// Validate network capability.
    fn validate_network(&self, network: &NetworkCapability) -> Result<(), RuntimeModuleError> {
        match network {
            NetworkCapability::None => Ok(()),
            NetworkCapability::Internal { .. } => {
                // Internal network is always allowed; CIDR enforcement
                // happens at the container level, not here.
                Ok(())
            }
            NetworkCapability::External { domains } => {
                for domain in domains {
                    if !self.config.allowed_domains.contains(domain) {
                        return Err(RuntimeModuleError::CapabilityDenied {
                            capability: format!(
                                "network: external domain '{domain}' not whitelisted"
                            ),
                        });
                    }
                }
                Ok(())
            }
            NetworkCapability::Full => {
                // Full network access is denied unless all domains are whitelisted.
                // In practice, Full should only be used internally.
                Err(RuntimeModuleError::CapabilityDenied {
                    capability: "network: full network access denied by policy".into(),
                })
            }
        }
    }

    /// Validate filesystem capability.
    fn validate_filesystem(
        &self,
        filesystem: &FilesystemCapability,
    ) -> Result<(), RuntimeModuleError> {
        if filesystem.host_access && !self.config.allow_host_access {
            return Err(RuntimeModuleError::CapabilityDenied {
                capability: "filesystem: host access denied by policy".into(),
            });
        }

        for mount in &filesystem.mounts {
            if !self.config.allowed_mounts.contains(&mount.host_path) {
                return Err(RuntimeModuleError::CapabilityDenied {
                    capability: format!(
                        "filesystem: mount '{}' not in allowed mounts",
                        mount.host_path
                    ),
                });
            }
        }

        Ok(())
    }

    /// Validate container capability and image whitelist.
    fn validate_container(
        &self,
        container: &ContainerCapability,
        request_image: Option<&str>,
    ) -> Result<(), RuntimeModuleError> {
        // Validate images from the capability declaration.
        for image in &container.allowed_images {
            if !self.config.image_whitelist.contains(image) {
                return Err(RuntimeModuleError::ImageNotAllowed {
                    image: image.clone(),
                });
            }
        }

        // Validate the request-level image.
        if let Some(image) = request_image {
            if !self.config.image_whitelist.contains(image) {
                return Err(RuntimeModuleError::ImageNotAllowed {
                    image: image.to_string(),
                });
            }
        }

        // Validate pull permission.
        if container.allow_pull && !self.config.allow_image_pull {
            return Err(RuntimeModuleError::CapabilityDenied {
                capability: "container: image pull denied by policy".into(),
            });
        }

        Ok(())
    }

    /// Validate process capability.
    fn validate_process(&self, process: &ProcessCapability) -> Result<(), RuntimeModuleError> {
        if process.shell && !self.config.allow_shell {
            return Err(RuntimeModuleError::CapabilityDenied {
                capability: "process: shell execution denied by policy".into(),
            });
        }
        Ok(())
    }

    /// Cap resource limits to the configured maximums.
    fn cap_resources(&self, caps: &RuntimeCapability) -> RuntimeCapability {
        let resources = &caps.container.resources;

        let memory_bytes = resources
            .memory_bytes
            .map(|m| m.min(self.config.default_memory_bytes));
        let cpu_quota = resources
            .cpu_quota
            .map(|c| c.min(self.config.default_cpu_quota));
        let timeout = resources
            .timeout
            .map(|t| t.min(self.config.default_timeout));
        let max_output_bytes = resources
            .max_output_bytes
            .map(|o| o.min(self.config.default_max_output_bytes));

        RuntimeCapability {
            network: caps.network.clone(),
            filesystem: caps.filesystem.clone(),
            container: ContainerCapability {
                allowed_images: caps.container.allowed_images.clone(),
                allow_pull: caps.container.allow_pull,
                resources: ResourceLimits {
                    memory_bytes,
                    cpu_quota,
                    timeout,
                    max_output_bytes,
                },
            },
            process: caps.process.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use y_core::runtime::{MountMode, MountSpec};

    use super::*;

    fn make_request(caps: RuntimeCapability) -> ExecutionRequest {
        ExecutionRequest {
            command: "echo".into(),
            args: vec!["hello".into()],
            working_dir: None,
            env: HashMap::new(),
            stdin: None,
            capabilities: caps,
            image: None,
        }
    }

    fn default_config() -> RuntimeConfig {
        RuntimeConfig::default()
    }

    // T-RT-001-01
    #[test]
    fn test_capability_no_network_allowed() {
        let config = default_config();
        let checker = CapabilityChecker::new(&config);
        let request = make_request(RuntimeCapability {
            network: NetworkCapability::None,
            ..Default::default()
        });
        assert!(checker.validate(&request).is_ok());
    }

    // T-RT-001-02
    #[test]
    fn test_capability_full_network_denied_by_policy() {
        let config = default_config();
        let checker = CapabilityChecker::new(&config);
        let request = make_request(RuntimeCapability {
            network: NetworkCapability::Full,
            ..Default::default()
        });
        let err = checker.validate(&request).unwrap_err();
        assert!(matches!(err, RuntimeModuleError::CapabilityDenied { .. }));
    }

    // T-RT-001-03
    #[test]
    fn test_capability_external_domains_validated() {
        let config = RuntimeConfig {
            allowed_domains: HashSet::from(["api.example.com".into()]),
            ..Default::default()
        };
        let checker = CapabilityChecker::new(&config);
        let request = make_request(RuntimeCapability {
            network: NetworkCapability::External {
                domains: vec!["api.example.com".into()],
            },
            ..Default::default()
        });
        assert!(checker.validate(&request).is_ok());
    }

    // T-RT-001-04
    #[test]
    fn test_capability_external_domain_blocked() {
        let config = RuntimeConfig {
            allowed_domains: HashSet::from(["api.example.com".into()]),
            ..Default::default()
        };
        let checker = CapabilityChecker::new(&config);
        let request = make_request(RuntimeCapability {
            network: NetworkCapability::External {
                domains: vec!["evil.com".into()],
            },
            ..Default::default()
        });
        let err = checker.validate(&request).unwrap_err();
        assert!(matches!(err, RuntimeModuleError::CapabilityDenied { .. }));
    }

    // T-RT-001-05
    #[test]
    fn test_capability_filesystem_mount_validated() {
        let config = RuntimeConfig {
            allowed_mounts: HashSet::from(["/data".into()]),
            ..Default::default()
        };
        let checker = CapabilityChecker::new(&config);
        let request = make_request(RuntimeCapability {
            filesystem: FilesystemCapability {
                mounts: vec![MountSpec {
                    host_path: "/data".into(),
                    container_path: "/mnt/data".into(),
                    mode: MountMode::ReadOnly,
                }],
                host_access: false,
            },
            ..Default::default()
        });
        assert!(checker.validate(&request).is_ok());
    }

    // T-RT-001-06
    #[test]
    fn test_capability_filesystem_host_access_denied() {
        let config = RuntimeConfig {
            allow_host_access: false,
            ..Default::default()
        };
        let checker = CapabilityChecker::new(&config);
        let request = make_request(RuntimeCapability {
            filesystem: FilesystemCapability {
                mounts: vec![],
                host_access: true,
            },
            ..Default::default()
        });
        let err = checker.validate(&request).unwrap_err();
        assert!(matches!(err, RuntimeModuleError::CapabilityDenied { .. }));
    }

    // T-RT-001-07
    #[test]
    fn test_capability_image_whitelist() {
        let config = RuntimeConfig {
            image_whitelist: HashSet::from(["python:3.11".into()]),
            ..Default::default()
        };
        let checker = CapabilityChecker::new(&config);
        let mut request = make_request(RuntimeCapability::default());
        request.image = Some("python:3.11".into());
        assert!(checker.validate(&request).is_ok());
    }

    // T-RT-001-08
    #[test]
    fn test_capability_image_not_whitelisted() {
        let config = RuntimeConfig {
            image_whitelist: HashSet::from(["python:3.11".into()]),
            ..Default::default()
        };
        let checker = CapabilityChecker::new(&config);
        let mut request = make_request(RuntimeCapability::default());
        request.image = Some("evil:latest".into());
        let err = checker.validate(&request).unwrap_err();
        assert!(matches!(err, RuntimeModuleError::ImageNotAllowed { .. }));
    }

    // T-RT-001-09
    #[test]
    fn test_capability_resource_limits_enforced() {
        let config = RuntimeConfig {
            default_memory_bytes: 512 * 1024 * 1024, // 512 MB cap
            ..Default::default()
        };
        let checker = CapabilityChecker::new(&config);
        let request = make_request(RuntimeCapability {
            container: ContainerCapability {
                resources: ResourceLimits {
                    memory_bytes: Some(2 * 1024 * 1024 * 1024), // 2 GB requested
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        });
        let result = checker.validate(&request).unwrap();
        // Should be capped to 512 MB.
        assert_eq!(
            result.container.resources.memory_bytes,
            Some(512 * 1024 * 1024)
        );
    }

    // T-RT-001-10
    #[test]
    fn test_capability_shell_denied() {
        let config = RuntimeConfig {
            allow_shell: false,
            ..Default::default()
        };
        let checker = CapabilityChecker::new(&config);
        let request = make_request(RuntimeCapability {
            process: ProcessCapability {
                shell: true,
                ..Default::default()
            },
            ..Default::default()
        });
        let err = checker.validate(&request).unwrap_err();
        assert!(matches!(err, RuntimeModuleError::CapabilityDenied { .. }));
    }
}
