//! Configuration re-exports.
//!
//! All configuration logic (`YAgentConfig`, `ConfigLoader`, `validate_config`,
//! `resolve_storage_paths`, `cleanup_old_logs`, path helpers) now lives in
//! `y_service::app_config`. This module re-exports them for backward
//! compatibility within `y-cli`.

pub use y_service::app_config::{
    cleanup_old_logs, dirs_log, dirs_user_config, home_dir, validate_config, ConfigLoader,
    YAgentConfig,
};
