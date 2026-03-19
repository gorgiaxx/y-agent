//! Chrome process launcher and lifecycle manager.
//!
//! Spawns a headless Chrome/Chromium instance with `--remote-debugging-port`
//! and manages its lifecycle. Used when `auto_launch = true` in `BrowserConfig`.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

/// Error type for Chrome launcher operations.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("Chrome executable not found. Set `chrome_path` in browser config or install Chrome/Chromium.")]
    ChromeNotFound,

    #[error("failed to spawn Chrome: {0}")]
    SpawnFailed(#[from] std::io::Error),

    #[error("Chrome CDP not ready after {0}s")]
    NotReady(u64),
}

/// Manages the lifecycle of a locally spawned Chrome process.
pub struct ChromeLauncher {
    child: Child,
    port: u16,
}

impl ChromeLauncher {
    /// Launch a Chrome instance.
    ///
    /// - `chrome_path`: explicit path, or empty to auto-detect.
    /// - `port`: preferred remote debugging port (if in use, a free port is chosen).
    /// - `headless`: when true, launches in headless mode (no visible window).
    ///
    /// Blocks until CDP is ready (polls `/json/version`).
    pub async fn launch(chrome_path: &str, port: u16, headless: bool) -> Result<Self, LaunchError> {
        let exe = if chrome_path.is_empty() {
            detect_chrome().ok_or(LaunchError::ChromeNotFound)?
        } else {
            PathBuf::from(chrome_path)
        };

        // Check if the preferred port is already in use.
        // If so, find a free port to avoid connecting to the wrong browser.
        let actual_port = if is_port_in_use(port) {
            let free = find_free_port().ok_or_else(|| {
                LaunchError::SpawnFailed(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("CDP port {port} is already in use and no free port could be found"),
                ))
            })?;
            warn!(
                preferred_port = port,
                actual_port = free,
                "CDP port {} already in use, using port {} instead",
                port,
                free
            );
            free
        } else {
            port
        };

        info!(path = %exe.display(), port = actual_port, "launching Chrome");

        // Create a temporary user-data-dir so multiple instances don't clash.
        let user_data_dir = std::env::temp_dir().join(format!("y-agent-chrome-{actual_port}"));
        std::fs::create_dir_all(&user_data_dir).ok();

        let mut cmd = Command::new(&exe);
        if headless {
            cmd.arg("--headless=new");
        }
        let child = cmd
            .arg(format!("--remote-debugging-port={actual_port}"))
            .arg(format!("--user-data-dir={}", user_data_dir.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-gpu")
            .arg("--disable-extensions")
            .arg("--disable-background-networking")
            .arg("--disable-sync")
            .arg("--disable-translate")
            .arg("--disable-session-crashed-bubble")
            .arg("--hide-crash-restore-bubble")
            .arg("--mute-audio")
            // Open a single about:blank tab — ensures exactly one page target
            // exists for resolve_ws_url to find, preventing duplicate windows.
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        let mut launcher = Self {
            child,
            port: actual_port,
        };

        // Wait for CDP to become ready.
        launcher.wait_ready(Duration::from_secs(15)).await?;

        info!(port, "Chrome CDP ready");
        Ok(launcher)
    }

    /// Poll `/json/version` until Chrome responds.
    async fn wait_ready(&mut self, timeout: Duration) -> Result<(), LaunchError> {
        let url = format!("http://127.0.0.1:{}/json/version", self.port);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            // Check that the child hasn't exited.
            if let Some(status) = self.child.try_wait().ok().flatten() {
                return Err(LaunchError::SpawnFailed(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Chrome exited early with status: {status}"),
                )));
            }

            match reqwest::get(&url).await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                _ => {}
            }

            if tokio::time::Instant::now() >= deadline {
                // Kill the process since it never became ready.
                self.shutdown().await;
                return Err(LaunchError::NotReady(timeout.as_secs()));
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// The CDP URL for this launched Chrome instance.
    pub fn cdp_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The actual CDP port this Chrome instance is using.
    ///
    /// May differ from the requested port if it was already in use.
    pub fn cdp_port(&self) -> u16 {
        self.port
    }

    /// Gracefully shutdown the Chrome process.
    pub async fn shutdown(&mut self) {
        debug!(port = self.port, "shutting down Chrome");

        // Try graceful kill first.
        #[cfg(unix)]
        {
            use nix::sys::signal::{self, Signal};
            use nix::unistd::Pid;

            if let Some(pid) = self.child.id() {
                let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
                // Give Chrome a moment to shut down gracefully.
                tokio::select! {
                    _ = self.child.wait() => {},
                    _ = tokio::time::sleep(Duration::from_secs(3)) => {
                        warn!("Chrome did not exit gracefully, force-killing");
                        let _ = self.child.kill().await;
                    }
                }
            }
        }

        #[cfg(not(unix))]
        {
            let _ = self.child.kill().await;
        }

        let _ = self.child.wait().await;
    }
}

impl Drop for ChromeLauncher {
    fn drop(&mut self) {
        // Best-effort synchronous kill. The `kill_on_drop(true)` on the
        // Command builder also ensures cleanup, but we try explicitly first.
        if let Some(pid) = self.child.id() {
            #[cfg(unix)]
            {
                use nix::sys::signal::{self, Signal};
                use nix::unistd::Pid;
                let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
            }
            #[cfg(not(unix))]
            {
                let _ = self.child.start_kill();
            }
            let _ = pid; // suppress unused warning on non-unix
        }
    }
}

/// Auto-detect Chrome/Chromium executable from well-known locations.
///
/// Order matters: prefer Google Chrome over Chromium over other Chromium-based
/// browsers. Brave is listed last because launching Brave with a custom
/// `--user-data-dir` while Brave is already open can cause the new instance
/// to merge into the existing one, leading to unexpected behavior.
fn detect_chrome() -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            // Brave last — launching while already open can cause tab-in-existing-window issues
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        ]
    } else {
        // Linux / other Unix
        &[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
        ]
    };

    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if path.exists() {
            debug!(path = %path.display(), "detected Chrome executable");
            return Some(path);
        }
        // For Linux, also check $PATH via `which`.
        if cfg!(not(target_os = "macos")) && cfg!(not(target_os = "windows")) {
            if let Ok(output) = std::process::Command::new("which").arg(candidate).output() {
                if output.status.success() {
                    let found = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !found.is_empty() {
                        debug!(path = %found, "detected Chrome via which");
                        return Some(PathBuf::from(found));
                    }
                }
            }
        }
    }

    None
}

/// Check if a TCP port is currently in use on 127.0.0.1.
fn is_port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

/// Find a free TCP port by binding to port 0.
fn find_free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}
