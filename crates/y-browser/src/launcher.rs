//! Chrome process launcher and lifecycle manager.
//!
//! Spawns a headless Chrome/Chromium instance with `--remote-debugging-port`
//! and manages its lifecycle. Used when `launch_mode` is `AutoLaunchHeadless`
//! or `AutoLaunchVisible` in `BrowserConfig`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::process::{Child, Command};
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::timeouts;

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
    temp_profile_dir: Option<PathBuf>,
}

impl ChromeLauncher {
    /// Launch a Chrome instance.
    ///
    /// - `chrome_path`: explicit path, or empty to auto-detect.
    /// - `port`: preferred remote debugging port (if in use, a free port is chosen).
    /// - `headless`: when true, launches in headless mode (no visible window).
    /// - `use_user_profile`: when true, uses the system user's default Chrome
    ///   profile instead of a clean temporary profile.
    ///
    /// Blocks until CDP is ready (polls `/json/version`).
    pub async fn launch(
        chrome_path: &str,
        port: u16,
        headless: bool,
        use_user_profile: bool,
    ) -> Result<Self, LaunchError> {
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

        let browser_flavor = BrowserFlavor::from_executable_path(&exe);

        let force_isolated_profile = should_force_isolated_profile(&exe, browser_flavor);
        let use_real_user_profile = use_user_profile && !force_isolated_profile;

        // Determine user-data-dir: real browser profile, managed persistent
        // profile, or an isolated temp dir.
        let (user_data_dir, temp_profile_dir) = if use_real_user_profile {
            if let Some(profile_dir) = detect_user_profile(&exe) {
                info!(
                    browser = browser_flavor.display_name(),
                    path = %profile_dir.display(),
                    "using system browser profile"
                );
                (profile_dir, None)
            } else {
                let profile_dir = create_managed_profile_dir(browser_flavor)?;
                warn!(
                    browser = browser_flavor.display_name(),
                    path = %profile_dir.display(),
                    "browser-specific user profile not found, using managed persistent browser profile"
                );
                info!(
                    browser = browser_flavor.display_name(),
                    path = %profile_dir.display(),
                    "using managed persistent browser profile"
                );
                (profile_dir, None)
            }
        } else {
            if use_user_profile && force_isolated_profile {
                let profile_dir = create_managed_profile_dir(browser_flavor)?;
                warn!(
                    browser = browser_flavor.display_name(),
                    path = %profile_dir.display(),
                    "Chrome 136+ ignores remote debugging on the default user profile; using a managed persistent browser profile instead"
                );
                info!(
                    browser = browser_flavor.display_name(),
                    path = %profile_dir.display(),
                    "using managed persistent browser profile"
                );
                (profile_dir, None)
            } else {
                let profile_dir = create_isolated_profile_dir(browser_flavor, actual_port)?;
                info!(
                    browser = browser_flavor.display_name(),
                    path = %profile_dir.display(),
                    "using isolated browser profile"
                );
                (profile_dir.clone(), Some(profile_dir))
            }
        };

        info!(
            browser = browser_flavor.display_name(),
            path = %exe.display(),
            port = actual_port,
            "launching Chrome"
        );

        let mut cmd = Command::new(&exe);
        if headless {
            cmd.arg("--headless=new");
        }
        cmd.arg(format!("--remote-debugging-port={actual_port}"))
            .arg(format!("--user-data-dir={}", user_data_dir.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            // .arg("--disable-gpu")
            .arg("--disable-background-networking")
            .arg("--disable-sync")
            .arg("--disable-translate")
            // .arg("--disable-session-crashed-bubble")
            // .arg("--hide-crash-restore-bubble")
            .arg("--mute-audio");

        // When NOT using the user profile, disable extensions for a clean env.
        // When using the user profile, keep extensions so the user's installed
        // extensions are available.
        if !use_real_user_profile {
            cmd.arg("--disable-extensions");
        }

        // Open a single about:blank tab -- ensures exactly one page target
        // exists for resolve_ws_url to find, preventing duplicate windows.
        let child = cmd
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        let mut launcher = Self {
            child,
            port: actual_port,
            temp_profile_dir,
        };

        // Wait for CDP to become ready.
        launcher.wait_ready(timeouts::LAUNCH_READY_WINDOW).await?;

        info!(port = actual_port, "Chrome CDP ready");
        Ok(launcher)
    }

    /// Poll `/json/version` until Chrome responds.
    async fn wait_ready(&mut self, timeout: Duration) -> Result<(), LaunchError> {
        let url = format!("http://127.0.0.1:{}/json/version", self.port);
        let deadline = tokio::time::Instant::now() + timeout;
        let mut exited_early = None;

        loop {
            if let Some(status) = self.child.try_wait().ok().flatten() {
                if exited_early.is_none() {
                    warn!(
                        port = self.port,
                        ?status,
                        "Chrome launcher process exited before CDP became ready; waiting briefly for a detached browser handoff"
                    );
                    exited_early = Some(status);
                }
            }

            if let Ok(resp) = reqwest::get(&url).await {
                if resp.status().is_success() {
                    return Ok(());
                }
            }

            if tokio::time::Instant::now() >= deadline {
                // Kill the process since it never became ready.
                self.shutdown().await;
                return if let Some(status) = exited_early {
                    Err(LaunchError::SpawnFailed(std::io::Error::other(format!(
                        "Chrome exited before CDP became ready (status: {status})"
                    ))))
                } else {
                    Err(LaunchError::NotReady(timeout.as_secs()))
                };
            }

            tokio::time::sleep(timeouts::LAUNCH_READY_POLL).await;
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

    /// Check whether the Chrome child process has exited.
    ///
    /// Returns `true` if the process has already exited (crash, user closed,
    /// etc.). This is non-blocking.
    pub fn child_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
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
                let _ = signal::kill(
                    Pid::from_raw(i32::try_from(pid).unwrap_or(i32::MAX)),
                    Signal::SIGTERM,
                );
                // Give Chrome a moment to shut down gracefully.
                tokio::select! {
                    _ = self.child.wait() => {},
                    () = tokio::time::sleep(timeouts::STOP_GRACEFUL) => {
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
        self.cleanup_temp_profile_dir();
    }

    /// Spawn a background task that waits for the Chrome process to exit
    /// and signals via a `watch` channel.
    ///
    /// The returned `Receiver` yields `true` once the process exits.
    /// This is the Rust equivalent of the openclaw `proc.on("exit", ...)` pattern.
    ///
    /// The watcher task is lightweight: it simply `await`s the child
    /// process and then sends a single notification.
    pub fn spawn_exit_watcher(&mut self) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        let child_id = self.child.id();
        let port = self.port;

        // We need a separate handle to wait on the child without consuming
        // the Child. tokio::process::Child::wait() requires &mut self,
        // so we use the child's PID to detect exit via a polling task.
        tokio::spawn(async move {
            // Poll at a reasonable interval. We cannot call child.wait()
            // here because we don't own the Child. Instead we check if
            // the process still exists.
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if let Some(pid) = child_id {
                    #[cfg(unix)]
                    {
                        // kill(pid, 0) checks if the process exists.
                        use nix::sys::signal;
                        use nix::unistd::Pid;
                        let alive = signal::kill(
                            Pid::from_raw(i32::try_from(pid).unwrap_or(i32::MAX)),
                            None,
                        )
                        .is_ok();
                        if !alive {
                            info!(port, pid, "Chrome process exited (detected by watcher)");
                            let _ = tx.send(true);
                            return;
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        // On non-Unix, fall back to checking if the port is
                        // still responding.
                        if !is_port_in_use(port) {
                            info!(port, pid, "Chrome port no longer in use");
                            let _ = tx.send(true);
                            return;
                        }
                    }
                } else {
                    // No PID available (shouldn't happen), just signal exit.
                    let _ = tx.send(true);
                    return;
                }
            }
        });

        rx
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
                let _ = signal::kill(
                    Pid::from_raw(i32::try_from(pid).unwrap_or(i32::MAX)),
                    Signal::SIGTERM,
                );
            }
            #[cfg(not(unix))]
            {
                let _ = self.child.start_kill();
            }
            let _ = pid; // suppress unused warning on non-unix
        }

        self.cleanup_temp_profile_dir();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserFlavor {
    GoogleChrome,
    ChromeCanary,
    Chromium,
    MicrosoftEdge,
    Brave,
    Unknown,
}

impl BrowserFlavor {
    fn from_executable_path(path: &Path) -> Self {
        let normalized = path.to_string_lossy().to_ascii_lowercase();

        if normalized.contains("brave") {
            Self::Brave
        } else if normalized.contains("chrome canary") || normalized.contains("chrome sxs") {
            Self::ChromeCanary
        } else if normalized.contains("microsoft edge") || normalized.contains("msedge") {
            Self::MicrosoftEdge
        } else if normalized.contains("chromium") {
            Self::Chromium
        } else if normalized.contains("google chrome")
            || normalized.ends_with("/chrome")
            || normalized.ends_with("\\chrome.exe")
            || normalized == "google-chrome"
            || normalized == "google-chrome-stable"
        {
            Self::GoogleChrome
        } else {
            Self::Unknown
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::GoogleChrome => "Google Chrome",
            Self::ChromeCanary => "Google Chrome Canary",
            Self::Chromium => "Chromium",
            Self::MicrosoftEdge => "Microsoft Edge",
            Self::Brave => "Brave Browser",
            Self::Unknown => "Chromium browser",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::GoogleChrome => "google-chrome",
            Self::ChromeCanary => "chrome-canary",
            Self::Chromium => "chromium",
            Self::MicrosoftEdge => "microsoft-edge",
            Self::Brave => "brave-browser",
            Self::Unknown => "chromium-browser",
        }
    }
}

fn should_force_isolated_profile(exe: &Path, flavor: BrowserFlavor) -> bool {
    should_force_isolated_profile_for_version(flavor, browser_major_version(exe))
}

fn should_force_isolated_profile_for_version(
    flavor: BrowserFlavor,
    major_version: Option<u32>,
) -> bool {
    matches!(
        flavor,
        BrowserFlavor::GoogleChrome | BrowserFlavor::ChromeCanary
    ) && major_version.is_some_and(|version| version >= 136)
}

fn browser_major_version(exe: &Path) -> Option<u32> {
    let output = std::process::Command::new(exe)
        .arg("--version")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_browser_major_version(&stdout)
}

fn parse_browser_major_version(version_output: &str) -> Option<u32> {
    version_output.split_whitespace().find_map(|token| {
        token
            .chars()
            .next()
            .filter(char::is_ascii_digit)
            .and_then(|_| token.split('.').next())
            .and_then(|major| major.parse::<u32>().ok())
    })
}

/// Auto-detect Chrome/Chromium executable from well-known locations.
///
/// Order matters: prefer Google Chrome over Chromium over other Chromium-based
/// browsers. Brave is listed last because launching Brave with a custom
/// `--user-data-dir` while Brave is already open can cause the new instance
/// to merge into the existing one, leading to unexpected behavior.
fn detect_chrome() -> Option<PathBuf> {
    let mut paths_to_check: Vec<PathBuf> = Vec::new();

    if cfg!(target_os = "macos") {
        let candidates = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            // Brave last — launching while already open can cause tab-in-existing-window issues
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ];
        paths_to_check.extend(candidates.into_iter().map(PathBuf::from));
    } else if cfg!(target_os = "windows") {
        let candidates = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
        ];
        paths_to_check.extend(candidates.into_iter().map(PathBuf::from));

        if let Ok(appdata) = std::env::var("APPDATA") {
            paths_to_check.push(PathBuf::from(&appdata).join(r"360se6\Application\360se.exe"));
        }
        if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
            paths_to_check
                .push(PathBuf::from(&localappdata).join(r"Google\Chrome\Application\chrome.exe"));
            paths_to_check
                .push(PathBuf::from(&localappdata).join(r"Microsoft\Edge\Application\msedge.exe"));
            paths_to_check.push(
                PathBuf::from(&localappdata)
                    .join(r"BraveSoftware\Brave-Browser\Application\brave.exe"),
            );
        }
    } else {
        // Linux / other Unix
        let candidates = [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
        ];
        paths_to_check.extend(candidates.into_iter().map(PathBuf::from));
    }

    for path in paths_to_check {
        if path.exists() {
            // Verify the executable is functional by running --version.
            // This catches residual installations where the binary exists
            // on disk but is broken (e.g. uninstalled Chrome leaving files behind).
            match std::process::Command::new(&path)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Ok(status) if status.success() => {
                    debug!(path = %path.display(), "detected Chrome executable");
                    return Some(path);
                }
                Ok(status) => {
                    warn!(
                        path = %path.display(),
                        exit_code = ?status.code(),
                        "Chrome candidate exists but --version failed, skipping"
                    );
                }
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "Chrome candidate exists but failed to execute, skipping"
                    );
                }
            }
        }
        // For Linux, also check $PATH via `which`.
        if cfg!(not(target_os = "macos")) && cfg!(not(target_os = "windows")) {
            // Only check `which` for candidates that are just executable names, not full paths.
            // This avoids trying to `which` something like "/usr/bin/google-chrome".
            // We assume that if `path` contains a path separator, it's a full path.
            if path
                .file_name()
                .is_some_and(|name| name == path.as_os_str())
            {
                if let Ok(output) = std::process::Command::new("which").arg(&path).output() {
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
    }

    None
}

/// Check if a TCP port is currently in use on 127.0.0.1.
pub(crate) fn is_port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

/// Find a free TCP port by binding to port 0.
pub(crate) fn find_free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

/// Ensure a port is available for use, similar to the openclaw `ensurePortAvailable`.
///
/// If the port is currently in use, checks whether it's a Chrome instance
/// by probing `/json/version`. Returns:
/// - `Ok(PortStatus::Available)` if the port is free.
/// - `Ok(PortStatus::ChromeRunning)` if Chrome is already listening on it.
/// - `Err(...)` if the port is occupied by something else.
pub async fn ensure_port_available(port: u16) -> Result<PortStatus, LaunchError> {
    if !is_port_in_use(port) {
        return Ok(PortStatus::Available);
    }

    // Port is in use -- check if it's Chrome.
    let url = format!("http://127.0.0.1:{port}/json/version");
    match tokio::time::timeout(timeouts::HEALTH_HTTP_TIMEOUT, reqwest::get(&url)).await {
        Ok(Ok(resp)) if resp.status().is_success() => {
            info!(port, "existing Chrome instance detected on port");
            Ok(PortStatus::ChromeRunning)
        }
        _ => Err(LaunchError::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            format!(
                "port {port} is in use by a non-Chrome process; \
                 cannot launch Chrome"
            ),
        ))),
    }
}

/// Result of a port availability check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortStatus {
    /// The port is free and can be used.
    Available,
    /// Chrome is already running on this port (can be reused).
    ChromeRunning,
}

/// Detect the default Chrome user-data directory for the current platform.
///
/// Falls back to a temp directory if the platform-specific path cannot be
/// determined (e.g. missing HOME env var).
fn detect_user_profile(exe: &Path) -> Option<PathBuf> {
    let profile_dir = default_user_profile_dir(BrowserFlavor::from_executable_path(exe))?;
    profile_dir.exists().then_some(profile_dir)
}

#[cfg(target_os = "macos")]
fn default_user_profile_dir(flavor: BrowserFlavor) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(macos_user_profile_dir(flavor, Path::new(&home)))
}

#[cfg(target_os = "windows")]
fn default_user_profile_dir(flavor: BrowserFlavor) -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(windows_user_profile_dir(flavor, Path::new(&local)))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn default_user_profile_dir(flavor: BrowserFlavor) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(linux_user_profile_dir(flavor, Path::new(&home)))
}

#[cfg(target_os = "macos")]
fn macos_user_profile_dir(flavor: BrowserFlavor, home: &Path) -> PathBuf {
    match flavor {
        BrowserFlavor::GoogleChrome | BrowserFlavor::Unknown => {
            home.join("Library/Application Support/Google/Chrome")
        }
        BrowserFlavor::ChromeCanary => {
            home.join("Library/Application Support/Google/Chrome Canary")
        }
        BrowserFlavor::Chromium => home.join("Library/Application Support/Chromium"),
        BrowserFlavor::MicrosoftEdge => home.join("Library/Application Support/Microsoft Edge"),
        BrowserFlavor::Brave => {
            home.join("Library/Application Support/BraveSoftware/Brave-Browser")
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_user_profile_dir(flavor: BrowserFlavor, local_app_data: &Path) -> PathBuf {
    match flavor {
        BrowserFlavor::GoogleChrome | BrowserFlavor::Unknown => {
            local_app_data.join(r"Google\Chrome\User Data")
        }
        BrowserFlavor::ChromeCanary => local_app_data.join(r"Google\Chrome SxS\User Data"),
        BrowserFlavor::Chromium => local_app_data.join(r"Chromium\User Data"),
        BrowserFlavor::MicrosoftEdge => local_app_data.join(r"Microsoft\Edge\User Data"),
        BrowserFlavor::Brave => local_app_data.join(r"BraveSoftware\Brave-Browser\User Data"),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn linux_user_profile_dir(flavor: BrowserFlavor, home: &Path) -> PathBuf {
    match flavor {
        BrowserFlavor::GoogleChrome | BrowserFlavor::Unknown => home.join(".config/google-chrome"),
        BrowserFlavor::ChromeCanary => home.join(".config/google-chrome-unstable"),
        BrowserFlavor::Chromium => home.join(".config/chromium"),
        BrowserFlavor::MicrosoftEdge => home.join(".config/microsoft-edge"),
        BrowserFlavor::Brave => home.join(".config/BraveSoftware/Brave-Browser"),
    }
}

fn create_isolated_profile_dir(flavor: BrowserFlavor, port: u16) -> Result<PathBuf, LaunchError> {
    let base = std::env::temp_dir();

    for attempt in 0..10 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = base.join(format!(
            "y-agent-{}-{port}-{nonce}-{attempt}",
            flavor.slug()
        ));

        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(LaunchError::SpawnFailed(err)),
        }
    }

    Err(LaunchError::SpawnFailed(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!(
            "failed to allocate a unique browser profile directory for {}",
            flavor.display_name()
        ),
    )))
}

fn create_managed_profile_dir(flavor: BrowserFlavor) -> Result<PathBuf, LaunchError> {
    let path = managed_profile_dir(flavor).ok_or_else(|| {
        LaunchError::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not resolve home directory for managed browser profile",
        ))
    })?;
    std::fs::create_dir_all(&path).map_err(LaunchError::SpawnFailed)?;
    Ok(path)
}

fn managed_profile_dir(flavor: BrowserFlavor) -> Option<PathBuf> {
    let home = home_dir()?;
    Some(managed_profile_dir_from_home(&home, flavor))
}

fn managed_profile_dir_from_home(home: &Path, flavor: BrowserFlavor) -> PathBuf {
    home.join(".config")
        .join("y-agent")
        .join("browser-profiles")
        .join(flavor.slug())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

impl ChromeLauncher {
    fn cleanup_temp_profile_dir(&mut self) {
        if let Some(path) = self.temp_profile_dir.take() {
            if let Err(error) = std::fs::remove_dir_all(&path) {
                warn!(
                    path = %path.display(),
                    %error,
                    "failed to clean up temporary browser profile directory"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_flavor_detects_known_executables() {
        assert_eq!(
            BrowserFlavor::from_executable_path(Path::new(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
            )),
            BrowserFlavor::GoogleChrome
        );
        assert_eq!(
            BrowserFlavor::from_executable_path(Path::new(
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"
            )),
            BrowserFlavor::Brave
        );
        assert_eq!(
            BrowserFlavor::from_executable_path(Path::new(
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
            )),
            BrowserFlavor::MicrosoftEdge
        );
        assert_eq!(
            BrowserFlavor::from_executable_path(Path::new(
                "/Applications/Chromium.app/Contents/MacOS/Chromium"
            )),
            BrowserFlavor::Chromium
        );
    }

    #[test]
    fn test_parse_browser_major_version_reads_chrome_and_brave_formats() {
        assert_eq!(
            parse_browser_major_version("Google Chrome 147.0.7727.56"),
            Some(147)
        );
        assert_eq!(
            parse_browser_major_version("Brave Browser 147.1.89.132"),
            Some(147)
        );
        assert_eq!(parse_browser_major_version("Chromium"), None);
    }

    #[test]
    fn test_force_isolated_profile_only_for_modern_google_chrome_variants() {
        assert!(should_force_isolated_profile_for_version(
            BrowserFlavor::GoogleChrome,
            Some(147)
        ));
        assert!(should_force_isolated_profile_for_version(
            BrowserFlavor::ChromeCanary,
            Some(136)
        ));
        assert!(!should_force_isolated_profile_for_version(
            BrowserFlavor::GoogleChrome,
            Some(135)
        ));
        assert!(!should_force_isolated_profile_for_version(
            BrowserFlavor::Brave,
            Some(147)
        ));
        assert!(!should_force_isolated_profile_for_version(
            BrowserFlavor::GoogleChrome,
            None
        ));
    }

    #[test]
    fn test_create_isolated_profile_dir_is_unique_and_browser_scoped() {
        let brave_dir = create_isolated_profile_dir(BrowserFlavor::Brave, 9222).unwrap();
        let chrome_dir = create_isolated_profile_dir(BrowserFlavor::GoogleChrome, 9222).unwrap();

        assert_ne!(brave_dir, chrome_dir);
        assert!(brave_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("brave-browser"));
        assert!(chrome_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("google-chrome"));

        std::fs::remove_dir_all(brave_dir).unwrap();
        std::fs::remove_dir_all(chrome_dir).unwrap();
    }

    #[test]
    fn test_managed_profile_dir_uses_y_agent_config_root() {
        let home = Path::new("/Users/tester");

        assert_eq!(
            managed_profile_dir_from_home(home, BrowserFlavor::GoogleChrome),
            PathBuf::from("/Users/tester/.config/y-agent/browser-profiles/google-chrome")
        );
        assert_eq!(
            managed_profile_dir_from_home(home, BrowserFlavor::Brave),
            PathBuf::from("/Users/tester/.config/y-agent/browser-profiles/brave-browser")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_user_profile_dirs_match_browser_brand() {
        let home = Path::new("/Users/tester");

        assert_eq!(
            macos_user_profile_dir(BrowserFlavor::GoogleChrome, home),
            PathBuf::from("/Users/tester/Library/Application Support/Google/Chrome")
        );
        assert_eq!(
            macos_user_profile_dir(BrowserFlavor::Brave, home),
            PathBuf::from("/Users/tester/Library/Application Support/BraveSoftware/Brave-Browser")
        );
        assert_eq!(
            macos_user_profile_dir(BrowserFlavor::MicrosoftEdge, home),
            PathBuf::from("/Users/tester/Library/Application Support/Microsoft Edge")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_windows_user_profile_dirs_match_browser_brand() {
        let local = Path::new(r"C:\Users\tester\AppData\Local");

        assert_eq!(
            windows_user_profile_dir(BrowserFlavor::GoogleChrome, local),
            PathBuf::from(r"C:\Users\tester\AppData\Local\Google\Chrome\User Data")
        );
        assert_eq!(
            windows_user_profile_dir(BrowserFlavor::Brave, local),
            PathBuf::from(r"C:\Users\tester\AppData\Local\BraveSoftware\Brave-Browser\User Data")
        );
        assert_eq!(
            windows_user_profile_dir(BrowserFlavor::MicrosoftEdge, local),
            PathBuf::from(r"C:\Users\tester\AppData\Local\Microsoft\Edge\User Data")
        );
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn test_linux_user_profile_dirs_match_browser_brand() {
        let home = Path::new("/home/tester");

        assert_eq!(
            linux_user_profile_dir(BrowserFlavor::GoogleChrome, home),
            PathBuf::from("/home/tester/.config/google-chrome")
        );
        assert_eq!(
            linux_user_profile_dir(BrowserFlavor::Brave, home),
            PathBuf::from("/home/tester/.config/BraveSoftware/Brave-Browser")
        );
        assert_eq!(
            linux_user_profile_dir(BrowserFlavor::MicrosoftEdge, home),
            PathBuf::from("/home/tester/.config/microsoft-edge")
        );
    }
}
