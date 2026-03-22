//! Platform-specific utilities for cross-platform runtime execution.
//!
//! Provides shell detection and other OS-specific helpers so that the
//! rest of the runtime crate can remain platform-agnostic.

/// Return the platform shell executable and argument flag for executing a
/// command string.
///
/// - **Unix** (Linux / macOS): `("sh", "-c")`
/// - **Windows**: `("cmd.exe", "/C")`
///
/// # Examples
///
/// ```
/// let (shell, flag) = y_runtime::platform::shell_command();
/// // On Unix: shell == "sh", flag == "-c"
/// // On Windows: shell == "cmd.exe", flag == "/C"
/// ```
pub fn shell_command() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("cmd.exe", "/C")
    } else {
        ("sh", "-c")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_command_returns_valid_pair() {
        let (shell, flag) = shell_command();
        assert!(!shell.is_empty());
        assert!(!flag.is_empty());

        #[cfg(unix)]
        {
            assert_eq!(shell, "sh");
            assert_eq!(flag, "-c");
        }

        #[cfg(windows)]
        {
            assert_eq!(shell, "cmd.exe");
            assert_eq!(flag, "/C");
        }
    }
}
