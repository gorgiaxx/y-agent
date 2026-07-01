//! Platform-specific utilities for cross-platform runtime execution.
//!
//! Re-exports [`y_core::platform`] so the runtime and tool layers share a
//! single source of truth for shell selection. See
//! [`y_core::platform::shell_command`] for details.

pub use y_core::platform::shell_command;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_command_delegates_to_core() {
        let (shell, flag) = shell_command();
        assert!(!shell.is_empty());
        assert!(!flag.is_empty());

        #[cfg(unix)]
        assert_eq!(flag, "-lc");

        #[cfg(windows)]
        assert_eq!(shell, "cmd.exe");
    }
}
