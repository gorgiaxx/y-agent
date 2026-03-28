//! Centralized timeout constants for Chrome/CDP operations.
//!
//! Keeping every timeout in one place makes it easy to find, reason about,
//! and tune values without hunting through multiple source files.

use std::time::Duration;

// -- Chrome launch -----------------------------------------------------------

/// Maximum time to wait for Chrome to respond on `/json/version` after launch.
pub const LAUNCH_READY_WINDOW: Duration = Duration::from_secs(15);

/// Polling interval while waiting for Chrome readiness after launch.
pub const LAUNCH_READY_POLL: Duration = Duration::from_millis(200);

/// How long to wait for Chrome to exit gracefully (SIGTERM) before SIGKILL.
pub const STOP_GRACEFUL: Duration = Duration::from_secs(3);

// -- CDP connection ----------------------------------------------------------

/// Timeout for CDP WebSocket handshake.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Delay between CDP connection retries.
pub const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Maximum number of CDP connection attempts.
pub const CONNECT_MAX_RETRIES: u32 = 3;

// -- CDP health probe --------------------------------------------------------

/// Timeout for an HTTP health request (e.g. `GET /json/version`).
pub const HEALTH_HTTP_TIMEOUT: Duration = Duration::from_millis(1500);

/// Timeout for a WebSocket health command (`Browser.getVersion`).
pub const HEALTH_WS_TIMEOUT: Duration = Duration::from_millis(800);

// -- CDP page target discovery -----------------------------------------------

/// Maximum retries when polling `/json/list` to find a page target.
pub const TARGET_DISCOVERY_MAX_RETRIES: u32 = 10;

/// Delay between `/json/list` retries.
pub const TARGET_DISCOVERY_RETRY_DELAY: Duration = Duration::from_millis(200);

/// Delay after `Target.createTarget` before querying `/json/list`.
pub const TARGET_CREATE_SETTLE: Duration = Duration::from_millis(200);

/// Timeout for the `Target.createTarget` CDP command response.
pub const TARGET_CREATE_TIMEOUT: Duration = Duration::from_secs(10);

// -- Default request timeout -------------------------------------------------

/// Default timeout for ordinary CDP requests when none is specified by config.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
