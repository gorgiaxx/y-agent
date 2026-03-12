//! Resource cleanup utilities for runtime backends.

use tracing::info;

use y_core::runtime::RuntimeAdapter;

/// Clean up resources for all provided runtime backends.
///
/// This is a convenience function that calls `cleanup()` on each backend
/// and logs any errors without failing. Used during graceful shutdown.
pub async fn cleanup_all(backends: &[&dyn RuntimeAdapter]) {
    for backend in backends {
        let backend_type = backend.backend();
        match backend.cleanup().await {
            Ok(()) => {
                info!(?backend_type, "runtime cleanup completed");
            }
            Err(e) => {
                tracing::warn!(?backend_type, error = %e, "runtime cleanup failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuntimeConfig;
    use crate::native::NativeRuntime;

    #[tokio::test]
    async fn test_cleanup_all_succeeds() {
        let native = NativeRuntime::new(RuntimeConfig::default(), None);
        cleanup_all(&[&native]).await;
        // No assertion needed — just verify it doesn't panic.
    }
}
