//! Virtual environment management for Python (uv) and JavaScript (bun).
//!
//! Provides [`VenvManager`] which validates and initialises virtual environments
//! at agent startup. The resulting status information is used by the prompt
//! layer to inject environment paths so the LLM runs scripts in the correct
//! environment.

use std::path::Path;
use std::process::Stdio;

use tokio::process::Command;

use crate::config::{BunVenvConfig, PythonVenvConfig, RuntimeConfig};

// ---------------------------------------------------------------------------
// Status types
// ---------------------------------------------------------------------------

/// Status of a single virtual environment after initialisation.
#[derive(Debug, Clone)]
pub struct VenvStatus {
    /// Whether the environment is ready for use.
    pub ready: bool,
    /// Resolved absolute path to the binary (uv / bun).
    pub binary_path: String,
    /// Detected version string.
    pub version: String,
    /// Human-readable message (e.g. error reason).
    pub message: String,
}

/// Aggregate report from [`VenvManager::init_all`].
#[derive(Debug, Clone)]
pub struct VenvInitReport {
    /// Python (uv) environment status (`None` if disabled).
    pub python: Option<VenvStatus>,
    /// JavaScript (bun) environment status (`None` if disabled).
    pub bun: Option<VenvStatus>,
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// Validates and optionally bootstraps virtual environments.
pub struct VenvManager;

impl VenvManager {
    /// Initialise the Python environment using `uv`.
    ///
    /// Steps:
    /// 1. Check that `uv` is accessible (via `which` / PATH lookup).
    /// 2. Query version (`uv --version`).
    /// 3. If `venv_dir` does not exist under the working directory, create it
    ///    with `uv venv --python <version> <venv_dir>`.
    pub async fn init_python(config: &PythonVenvConfig) -> VenvStatus {
        // 1. Find binary.
        let Some(binary_path) = Self::which(&config.uv_path).await else {
            return VenvStatus {
                ready: false,
                binary_path: config.uv_path.clone(),
                version: String::new(),
                message: format!("`{}` not found in PATH", config.uv_path),
            };
        };

        // 2. Query version.
        let version = Self::query_version(&binary_path, &["--version"])
            .await
            .unwrap_or_default();

        // 3. Ensure working directory exists.
        let work_dir = &config.working_dir;
        if let Err(e) = std::fs::create_dir_all(work_dir) {
            return VenvStatus {
                ready: false,
                binary_path,
                version,
                message: format!("failed to create working dir {work_dir}: {e}"),
            };
        }

        // 4. Ensure venv directory exists.
        let venv_path = Path::new(work_dir).join(&config.venv_dir);

        if !venv_path.exists() {
            tracing::info!(
                venv_dir = %venv_path.display(),
                python_version = %config.python_version,
                "creating Python venv via uv"
            );
            let result = Command::new(&binary_path)
                .args(["venv", "--python", &config.python_version, &config.venv_dir])
                .current_dir(work_dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await;

            match result {
                Ok(output) if output.status.success() => {
                    tracing::info!("Python venv created successfully");
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return VenvStatus {
                        ready: false,
                        binary_path,
                        version,
                        message: format!("uv venv failed: {stderr}"),
                    };
                }
                Err(e) => {
                    return VenvStatus {
                        ready: false,
                        binary_path,
                        version,
                        message: format!("failed to run uv venv: {e}"),
                    };
                }
            }
        }

        VenvStatus {
            ready: true,
            binary_path,
            version,
            message: "Python venv ready".into(),
        }
    }

    /// Initialise the JavaScript environment by validating `bun` is available.
    ///
    /// Unlike Python, bun does not require a separate venv creation step.
    pub async fn init_bun(config: &BunVenvConfig) -> VenvStatus {
        // 1. Find binary.
        let Some(binary_path) = Self::which(&config.bun_path).await else {
            return VenvStatus {
                ready: false,
                binary_path: config.bun_path.clone(),
                version: String::new(),
                message: format!("`{}` not found in PATH", config.bun_path),
            };
        };

        // 2. Query version.
        let version = Self::query_version(&binary_path, &["--version"])
            .await
            .unwrap_or_default();

        // 3. Ensure working directory exists.
        let work_dir = &config.working_dir;
        if let Err(e) = std::fs::create_dir_all(work_dir) {
            return VenvStatus {
                ready: false,
                binary_path,
                version,
                message: format!("failed to create working dir {work_dir}: {e}"),
            };
        }

        VenvStatus {
            ready: true,
            binary_path,
            version,
            message: "Bun environment ready".into(),
        }
    }

    /// Initialise all enabled virtual environments.
    pub async fn init_all(config: &RuntimeConfig) -> VenvInitReport {
        let python = if config.python_venv.enabled {
            let status = Self::init_python(&config.python_venv).await;
            tracing::info!(ready = status.ready, msg = %status.message, "Python venv init");
            Some(status)
        } else {
            None
        };

        let bun = if config.bun_venv.enabled {
            let status = Self::init_bun(&config.bun_venv).await;
            tracing::info!(ready = status.ready, msg = %status.message, "Bun venv init");
            Some(status)
        } else {
            None
        };

        VenvInitReport { python, bun }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Locate a binary on `PATH`, returning its absolute path.
    async fn which(name: &str) -> Option<String> {
        // On Unix, use the `which` command; on Windows, use `where`.
        let cmd = if cfg!(windows) { "where" } else { "which" };
        let output = Command::new(cmd)
            .arg(name)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
            .ok()?;

        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()?
                .trim()
                .to_string();
            if path.is_empty() {
                None
            } else {
                Some(path)
            }
        } else {
            None
        }
    }

    /// Query a binary for its version string.
    async fn query_version(binary: &str, args: &[&str]) -> Option<String> {
        let output = Command::new(binary)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .ok()?;

        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_python_config() {
        let config = PythonVenvConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.uv_path, "uv");
        assert_eq!(config.python_version, "3.12");
        assert_eq!(config.venv_dir, ".venv");
        assert!(config.working_dir.contains(".local/state/y-agent"));
    }

    #[test]
    fn test_default_bun_config() {
        let config = BunVenvConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.bun_path, "bun");
        assert_eq!(config.bun_version, "latest");
        assert!(config.working_dir.contains(".local/state/y-agent"));
    }

    #[tokio::test]
    async fn test_init_python_disabled() {
        let config = RuntimeConfig::default();
        let report = VenvManager::init_all(&config).await;
        assert!(report.python.is_none());
        assert!(report.bun.is_none());
    }

    #[tokio::test]
    async fn test_init_python_missing_binary() {
        let config = PythonVenvConfig {
            enabled: true,
            uv_path: "nonexistent-uv-binary-xyz".into(),
            ..Default::default()
        };
        let status = VenvManager::init_python(&config).await;
        assert!(!status.ready);
        assert!(status.message.contains("not found in PATH"));
    }

    #[tokio::test]
    async fn test_init_bun_missing_binary() {
        let config = BunVenvConfig {
            enabled: true,
            bun_path: "nonexistent-bun-binary-xyz".into(),
            ..Default::default()
        };
        let status = VenvManager::init_bun(&config).await;
        assert!(!status.ready);
        assert!(status.message.contains("not found in PATH"));
    }
}
