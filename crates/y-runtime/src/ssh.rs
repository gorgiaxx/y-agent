//! SSH runtime: remote command execution via the system `ssh` binary.
//!
//! Uses `tokio::process::Command` to invoke the system's `ssh` program,
//! which provides:
//! - Zero external Rust crate dependencies
//! - Full support for all auth methods (keys, passwords, agents, certificates)
//! - Respect for user's `~/.ssh/config` and known hosts
//! - Battle-tested SSH protocol implementation
//!
//! The runtime constructs SSH commands from `SshConfig` and delegates
//! to the system SSH client for actual connection and execution.

use std::time::Duration;

use async_trait::async_trait;

use y_core::runtime::{
    ExecutionRequest, ExecutionResult, ResourceUsage, RuntimeAdapter, RuntimeBackend, RuntimeError,
    RuntimeHealth,
};

use crate::config::{SshAuthMethod, SshConfig};

/// SSH runtime backend using the system `ssh` binary.
pub struct SshRuntime {
    config: SshConfig,
}

impl SshRuntime {
    /// Create a new SSH runtime with the given configuration.
    pub fn new(config: SshConfig) -> Self {
        Self { config }
    }

    /// Build the `ssh` command with appropriate flags from config.
    fn build_ssh_command(&self, remote_command: &str) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("ssh");

        // Connection target.
        cmd.arg("-p").arg(self.config.port.to_string());

        // Disable pseudo-terminal allocation (we want raw stdout/stderr).
        cmd.arg("-T");

        // Batch mode: don't ask for passwords interactively.
        cmd.arg("-o").arg("BatchMode=yes");

        // Disable strict host key checking if no known_hosts file is set.
        // This mirrors the behavior of the previous `ServerCheckMethod::NoCheck`.
        if self.config.known_hosts_path.is_none() {
            cmd.arg("-o").arg("StrictHostKeyChecking=no");
            cmd.arg("-o").arg("UserKnownHostsFile=/dev/null");
        } else if let Some(ref kh) = self.config.known_hosts_path {
            cmd.arg("-o").arg(format!("UserKnownHostsFile={}", kh));
        }

        // Auth method.
        match self.config.auth_method {
            SshAuthMethod::PublicKey => {
                if let Some(ref key_path) = self.config.private_key_path {
                    cmd.arg("-i").arg(key_path);
                }
                // If no key path, ssh will use the default key or agent.
            }
            SshAuthMethod::Password => {
                // System SSH doesn't support password via command line flag.
                // Users should use `sshpass` or key-based auth.
                // BatchMode=yes will cause it to fail if password is needed,
                // which is the correct security behavior.
                tracing::warn!(
                    "SSH password auth is configured but the system ssh binary \
                     does not support direct password injection. \
                     Use key-based auth or sshpass."
                );
            }
        }

        // Suppress SSH banners and warnings.
        cmd.arg("-o").arg("LogLevel=ERROR");

        // User@Host
        let target = format!("{}@{}", self.config.user, self.config.host);
        cmd.arg(&target);

        // The remote command.
        cmd.arg(remote_command);

        // Capture output.
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        cmd
    }
}

impl Default for SshRuntime {
    fn default() -> Self {
        Self::new(SshConfig::default())
    }
}

#[async_trait]
impl RuntimeAdapter for SshRuntime {
    fn name(&self) -> &'static str {
        "ssh"
    }

    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        let start = std::time::Instant::now();

        // Build the full command string.
        let full_command = if request.args.is_empty() {
            request.command.clone()
        } else {
            format!("{} {}", request.command, request.args.join(" "))
        };

        // Prepend `cd <working_dir> &&` if specified.
        let full_command = if let Some(ref wd) = request.working_dir {
            format!("cd {} && {}", shell_escape(wd), full_command)
        } else {
            full_command
        };

        // Prepend environment variables.
        let full_command = if request.env.is_empty() {
            full_command
        } else {
            let env_prefix: String = request
                .env
                .iter()
                .map(|(k, v)| format!("{}={}", k, shell_escape(v)))
                .collect::<Vec<_>>()
                .join(" ");
            format!("{} {}", env_prefix, full_command)
        };

        // Get timeout from capabilities or use default.
        let timeout = request
            .capabilities
            .container
            .resources
            .timeout
            .unwrap_or(Duration::from_secs(120));

        let mut cmd = self.build_ssh_command(&full_command);

        tracing::info!(
            host = %self.config.host,
            port = self.config.port,
            command = %full_command,
            "executing command via SSH"
        );

        let result = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| RuntimeError::Other {
                message: format!("SSH command timed out after {timeout:?}: {full_command}"),
            })?
            .map_err(|e| RuntimeError::Other {
                message: format!(
                    "failed to spawn ssh process: {e}. \
                     Is the `ssh` command available in PATH?"
                ),
            })?;

        let duration = start.elapsed();
        let exit_code = result.status.code().unwrap_or(-1);

        tracing::debug!(
            exit_code,
            duration_ms = u64::try_from(duration.as_millis()).unwrap_or(0),
            "SSH command completed"
        );

        Ok(ExecutionResult {
            exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
            duration,
            resource_usage: ResourceUsage::default(),
        })
    }

    async fn health_check(&self) -> Result<RuntimeHealth, RuntimeError> {
        // Quick connectivity check: run `echo ok` on the remote host.
        let mut cmd = self.build_ssh_command("echo ok");

        // Use a short timeout for health check.
        let result = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await;

        match result {
            Ok(Ok(output)) if output.status.success() => Ok(RuntimeHealth {
                backend: RuntimeBackend::Ssh,
                available: true,
                message: Some(format!(
                    "SSH connected to {}@{}:{}",
                    self.config.user, self.config.host, self.config.port
                )),
            }),
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Ok(RuntimeHealth {
                    backend: RuntimeBackend::Ssh,
                    available: false,
                    message: Some(format!(
                        "SSH health check failed (exit {}): {}",
                        output.status.code().unwrap_or(-1),
                        stderr.trim()
                    )),
                })
            }
            Ok(Err(e)) => Ok(RuntimeHealth {
                backend: RuntimeBackend::Ssh,
                available: false,
                message: Some(format!("SSH health check failed: {e}. Is `ssh` in PATH?")),
            }),
            Err(_) => Ok(RuntimeHealth {
                backend: RuntimeBackend::Ssh,
                available: false,
                message: Some("SSH health check timed out (10s)".into()),
            }),
        }
    }

    fn backend(&self) -> RuntimeBackend {
        RuntimeBackend::Ssh
    }

    async fn cleanup(&self) -> Result<(), RuntimeError> {
        // No persistent connection to clean up — each command spawns a new process.
        Ok(())
    }
}

/// Escape a string for safe use in remote shell commands.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SshAuthMethod;

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("hello"), "'hello'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
        assert_eq!(shell_escape("a b c"), "'a b c'");
    }

    #[test]
    fn test_build_ssh_command_basic() {
        let config = SshConfig {
            host: "example.com".into(),
            port: 2222,
            user: "testuser".into(),
            auth_method: SshAuthMethod::PublicKey,
            private_key_path: Some("/path/to/key".into()),
            ..SshConfig::default()
        };
        let rt = SshRuntime::new(config);
        let cmd = rt.build_ssh_command("echo hello");

        // Verify the ssh command is constructed correctly.
        let program = cmd.as_std().get_program().to_string_lossy().to_string();
        assert_eq!(program, "ssh");

        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"2222".to_string()));
        assert!(args.contains(&"-T".to_string()));
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"/path/to/key".to_string()));
        assert!(args.contains(&"testuser@example.com".to_string()));
        assert!(args.contains(&"echo hello".to_string()));
    }

    #[test]
    fn test_build_ssh_command_with_known_hosts() {
        let config = SshConfig {
            known_hosts_path: Some("/custom/known_hosts".into()),
            ..SshConfig::default()
        };
        let rt = SshRuntime::new(config);
        let cmd = rt.build_ssh_command("ls");
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        assert!(args.contains(&"UserKnownHostsFile=/custom/known_hosts".to_string()));
        // Should NOT contain StrictHostKeyChecking=no
        assert!(!args.contains(&"StrictHostKeyChecking=no".to_string()));
    }

    #[test]
    fn test_default_runtime() {
        let rt = SshRuntime::default();
        assert_eq!(rt.config.host, "localhost");
        assert_eq!(rt.config.port, 22);
        assert_eq!(rt.name(), "ssh");
        assert_eq!(rt.backend(), RuntimeBackend::Ssh);
    }
}
