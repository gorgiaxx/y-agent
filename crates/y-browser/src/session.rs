//! Browser session manager -- connection caching and lifecycle.
//!
//! Inspired by the openclaw `cachedByCdpUrl` / `connectingByCdpUrl` pattern
//! (see `pw-session.ts`). The [`BrowserSession`] owns the Chrome process
//! (if auto-launched) and the CDP connection, and exposes them to the
//! rest of the crate.
//!
//! # Key behaviours
//!
//! * **Connection dedup**: concurrent `ensure_connected()` calls share a
//!   single connection future instead of racing.
//! * **Process exit watcher**: when auto-launched Chrome exits, the session
//!   transitions to `Disconnected` immediately (no polling required).
//! * **Shutdown hook**: `shutdown()` tears down both the CDP connection and
//!   the Chrome process for clean application exit.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::sync::{watch, Mutex, Notify};
use tracing::{debug, info, warn};

use crate::actions::BrowserActions;
use crate::cdp_client::CdpClient;
use crate::config::BrowserConfig;
use crate::launcher::{ensure_port_available, ChromeLauncher, PortStatus};
use crate::security::SecurityPolicy;
use crate::timeouts;

use y_core::tool::ToolError;

// -- Session state machine ---------------------------------------------------

const STATE_DISCONNECTED: u8 = 0;
const STATE_CONNECTING: u8 = 1;
const STATE_CONNECTED: u8 = 2;

/// Manages the browser connection lifecycle.
///
/// Wraps [`CdpClient`], [`ChromeLauncher`], [`BrowserActions`], and
/// [`SecurityPolicy`] behind a single owner that handles connection
/// establishment, health monitoring, and shutdown.
pub struct BrowserSession {
    client: Arc<CdpClient>,
    actions: BrowserActions,
    config: RwLock<BrowserConfig>,
    security: RwLock<SecurityPolicy>,
    launcher: Mutex<Option<ChromeLauncher>>,
    state: AtomicU8,
    /// Notifier for connection-in-progress dedup.
    connect_notify: Notify,
    /// Receiver for the Chrome process exit watcher.
    exit_watcher: Mutex<Option<watch::Receiver<bool>>>,
    /// Whether console monitoring has been started for the current connection.
    console_started: Mutex<bool>,
}

impl BrowserSession {
    /// Create a new session (not yet connected).
    pub fn new(config: BrowserConfig) -> Self {
        let cdp_url = if config.launch_mode.is_auto_launch() {
            format!("http://127.0.0.1:{}", config.local_cdp_port)
        } else {
            config.cdp_url.clone()
        };

        let client = Arc::new(CdpClient::new(
            cdp_url,
            Duration::from_millis(config.timeout_ms),
        ));
        let actions = BrowserActions::new(Arc::clone(&client));
        let security = SecurityPolicy::new(
            config.allowed_domains.clone(),
            config.block_private_networks,
        );

        Self {
            client,
            actions,
            config: RwLock::new(config),
            security: RwLock::new(security),
            launcher: Mutex::new(None),
            state: AtomicU8::new(STATE_DISCONNECTED),
            connect_notify: Notify::new(),
            exit_watcher: Mutex::new(None),
            console_started: Mutex::new(false),
        }
    }

    /// Hot-reload the browser configuration.
    ///
    /// Updates the stored config and rebuilds the security policy.
    /// Does NOT affect an already-running Chrome session; changes
    /// take effect on the next connection.
    ///
    /// # Panics
    ///
    /// Panics if the internal locks are poisoned.
    pub fn reload_config(&self, new_config: BrowserConfig) {
        let new_security = SecurityPolicy::new(
            new_config.allowed_domains.clone(),
            new_config.block_private_networks,
        );
        *self.security.write().unwrap() = new_security;
        *self.config.write().unwrap() = new_config;
        info!("Browser config hot-reloaded");
    }

    /// Reference to the shared `CdpClient`.
    pub fn client(&self) -> &Arc<CdpClient> {
        &self.client
    }

    /// Reference to the high-level browser actions.
    pub fn actions(&self) -> &BrowserActions {
        &self.actions
    }

    /// Read the current security policy.
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    pub fn security(&self) -> std::sync::RwLockReadGuard<'_, SecurityPolicy> {
        self.security.read().unwrap()
    }

    /// Read the current config.
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    pub fn config(&self) -> std::sync::RwLockReadGuard<'_, BrowserConfig> {
        self.config.read().unwrap()
    }

    /// Whether the session is connected.
    pub fn is_connected(&self) -> bool {
        self.state.load(Ordering::SeqCst) == STATE_CONNECTED
    }

    // -- Connection management -----------------------------------------------

    /// Ensure the CDP connection is established.
    ///
    /// If auto-launch is enabled and no Chrome is running, spawns one.
    /// If another task is already connecting, waits for it instead of
    /// starting a parallel connection (dedup, like openclaw's
    /// `connectingByCdpUrl`).
    pub async fn ensure_connected(&self) -> Result<(), ToolError> {
        // Fast path: already connected and still healthy.
        if self.state.load(Ordering::SeqCst) == STATE_CONNECTED {
            // Check process exit watcher for auto-launched Chrome.
            if self.check_exit_watcher().await {
                warn!("Chrome process exited, resetting session");
                self.reset().await;
            } else if self.client.is_connected().await {
                return Ok(());
            } else {
                // WebSocket died but process didn't exit yet.
                warn!("CDP connection lost, resetting session");
                self.reset().await;
            }
        }

        // Dedup: if someone else is already connecting, wait for them.
        if self
            .state
            .compare_exchange(
                STATE_DISCONNECTED,
                STATE_CONNECTING,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            // Another task is connecting or connected; wait up to the
            // connect timeout for it to finish.
            debug!("waiting for in-flight connection attempt");
            tokio::select! {
                () = self.connect_notify.notified() => {},
                () = tokio::time::sleep(timeouts::CONNECT_TIMEOUT) => {},
            }
            return if self.state.load(Ordering::SeqCst) == STATE_CONNECTED {
                Ok(())
            } else {
                Err(ToolError::ExternalServiceError {
                    name: "browser".into(),
                    message: "concurrent connection attempt failed".into(),
                })
            };
        }

        // We own the connecting state -- do the actual work.
        let result = self.do_connect().await;

        if result.is_ok() {
            self.state.store(STATE_CONNECTED, Ordering::SeqCst);
        } else {
            self.state.store(STATE_DISCONNECTED, Ordering::SeqCst);
        }
        // Wake all waiters.
        self.connect_notify.notify_waiters();
        result
    }

    /// Internal connection logic with retry.
    async fn do_connect(&self) -> Result<(), ToolError> {
        let config = self.config.read().unwrap().clone();

        if config.launch_mode.is_auto_launch() {
            self.connect_auto_launch(&config).await
        } else {
            self.connect_remote(&config).await
        }
    }

    /// Connect with auto-launch: ensure Chrome is running, then connect.
    async fn connect_auto_launch(&self, config: &BrowserConfig) -> Result<(), ToolError> {
        let mut launcher_guard = self.launcher.lock().await;

        if launcher_guard.is_none() {
            // Check port status before launching.
            let port = config.local_cdp_port;
            match ensure_port_available(port).await {
                Ok(PortStatus::ChromeRunning) => {
                    // Reuse existing Chrome -- just connect to it.
                    info!(port, "reusing existing Chrome on port");
                    let cdp_url = format!("http://127.0.0.1:{port}");
                    self.client.set_cdp_url(cdp_url);
                    drop(launcher_guard);
                    return self.connect_cdp_with_retry().await;
                }
                Ok(PortStatus::Available) => {
                    // Port is free, proceed with launch.
                }
                Err(e) => {
                    // Port occupied by non-Chrome. Try a free port.
                    warn!(port, error = %e, "configured port unavailable, finding free port");
                }
            }

            debug!("auto-launching Chrome");
            let mut chrome = ChromeLauncher::launch(
                &config.chrome_path,
                config.local_cdp_port,
                config.launch_mode.is_headless(),
                config.use_user_profile,
            )
            .await
            .map_err(|e| ToolError::ExternalServiceError {
                name: "browser".into(),
                message: format!("failed to launch Chrome: {e}"),
            })?;

            // Set up exit watcher.
            let exit_rx = chrome.spawn_exit_watcher();
            *self.exit_watcher.lock().await = Some(exit_rx);

            // Update client URL to match actual port.
            let actual_port = chrome.cdp_port();
            let cdp_url = format!("http://127.0.0.1:{actual_port}");
            self.client.set_cdp_url(cdp_url);

            *launcher_guard = Some(chrome);
        } else {
            // Launcher exists but connection was lost. Update URL and reconnect.
            let actual_port = launcher_guard.as_ref().unwrap().cdp_port();
            let cdp_url = format!("http://127.0.0.1:{actual_port}");
            self.client.set_cdp_url(cdp_url);
        }

        drop(launcher_guard);
        self.connect_cdp_with_retry().await
    }

    /// Connect to a remote CDP endpoint.
    async fn connect_remote(&self, config: &BrowserConfig) -> Result<(), ToolError> {
        let cdp_url = &config.cdp_url;
        debug!(cdp_url, "connecting to remote CDP");
        self.connect_cdp_with_retry()
            .await
            .map_err(|e| ToolError::ExternalServiceError {
                name: "browser".into(),
                message: format!(
                    "{e}. Make sure Chrome is running with \
                     --remote-debugging-port=9222"
                ),
            })
    }

    /// Connect CDP client with retry logic, mirroring openclaw's
    /// `connectWithRetry` pattern.
    async fn connect_cdp_with_retry(&self) -> Result<(), ToolError> {
        let mut last_err = None;

        for attempt in 0..timeouts::CONNECT_MAX_RETRIES {
            match self.client.connect().await {
                Ok(()) => {
                    // Start console monitoring on first connection.
                    let mut started = self.console_started.lock().await;
                    if !*started {
                        self.actions.enable_console_monitoring().await;
                        *started = true;
                    }
                    return Ok(());
                }
                Err(e) => {
                    let msg = e.to_string();
                    warn!(attempt, error = %msg, "CDP connect attempt failed");
                    last_err = Some(msg);

                    if attempt < timeouts::CONNECT_MAX_RETRIES - 1 {
                        let delay = timeouts::CONNECT_RETRY_DELAY
                            + Duration::from_millis(u64::from(attempt) * 250);
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(ToolError::ExternalServiceError {
            name: "browser".into(),
            message: format!(
                "failed to connect to Chrome CDP after {} attempts: {}",
                timeouts::CONNECT_MAX_RETRIES,
                last_err.unwrap_or_default()
            ),
        })
    }

    // -- Lifecycle -----------------------------------------------------------

    /// Check the exit watcher. Returns `true` if Chrome has exited.
    async fn check_exit_watcher(&self) -> bool {
        let guard = self.exit_watcher.lock().await;
        if let Some(ref rx) = *guard {
            *rx.borrow()
        } else {
            false
        }
    }

    /// Reset the session: disconnect CDP, tear down Chrome (if owned),
    /// and clear all state. The next `ensure_connected()` will start fresh.
    pub async fn reset(&self) {
        debug!("resetting browser session");
        self.state.store(STATE_DISCONNECTED, Ordering::SeqCst);
        self.client.disconnect().await;

        let mut launcher_guard = self.launcher.lock().await;
        if let Some(mut chrome) = launcher_guard.take() {
            chrome.shutdown().await;
        }

        *self.exit_watcher.lock().await = None;
        *self.console_started.lock().await = false;
    }

    /// Gracefully shut down the session (for application exit).
    ///
    /// This is the method that should be called from `y-service` during
    /// graceful shutdown to prevent zombie Chrome processes.
    pub async fn shutdown(&self) {
        info!("shutting down browser session");
        self.reset().await;
    }

    /// Check whether an error indicates a broken connection that can be
    /// recovered by reconnecting.
    pub fn is_connection_error(err: &ToolError) -> bool {
        match err {
            ToolError::ExternalServiceError { message, .. } => {
                let m = message.to_lowercase();
                m.contains("websocket")
                    || m.contains("closed connection")
                    || m.contains("connection lost")
                    || m.contains("not connected")
                    || m.contains("cdp connection")
            }
            _ => false,
        }
    }
}

impl Default for BrowserSession {
    fn default() -> Self {
        Self::new(BrowserConfig::default())
    }
}
