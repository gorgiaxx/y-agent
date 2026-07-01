//! Platform-specific helpers shared across crates.
//!
//! Centralizes shell detection so the runtime and tool layers agree on how a
//! shell command string is executed. The decision lives in `y-core` (which both
//! `y-runtime` and `y-tools` already depend on) instead of being duplicated.

/// Resolve the shell executable and argument flag for executing a command
/// string as a **login shell**.
///
/// On Unix the user's login shell (`$SHELL`) is preferred so that the correct
/// profile files are sourced — `~/.zprofile` for zsh, `~/.bash_profile` /
/// `~/.profile` for bash/sh. This matters because package managers (Homebrew on
/// Apple Silicon, in particular) write their `PATH` setup into shell-specific
/// profile files: `brew shellenv` lands in `~/.zprofile`, which `sh` never
/// reads. When the host process was launched with a minimal environment (the
/// common case for GUI-launched hosts such as Tauri, Dock, or Finder, where
/// launchd does not provide a login environment), a plain `sh -lc` would
/// reconstruct most of `PATH` but still miss everything wired up only in the
/// user's real login-shell profile. Invoking the user's own shell as a login
/// shell rebuilds the full environment (PATH, HOME, `CARGO_HOME`, library
/// paths, brew, ...).
///
/// Falls back to `sh -lc` when `$SHELL` is unset, empty, or points at a
/// non-existent binary.
///
/// - **Unix**: `($SHELL, "-lc")` (default `("sh", "-lc")`)
/// - **Windows**: `("cmd.exe", "/C")`
///
/// # Examples
///
/// ```
/// let (shell, flag) = y_core::platform::shell_command();
/// assert!(!shell.is_empty());
/// assert!(!flag.is_empty());
/// ```
pub fn shell_command() -> (String, String) {
    if cfg!(windows) {
        return ("cmd.exe".into(), "/C".into());
    }

    // Prefer the user's real login shell so shell-specific profile files
    // (e.g. ~/.zprofile with `brew shellenv`) are sourced.
    if let Some(shell) = user_login_shell() {
        return (shell, "-lc".into());
    }

    ("sh".into(), "-lc".into())
}

/// Read `$SHELL` and keep it only if the binary actually exists.
fn user_login_shell() -> Option<String> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty())?;
    if std::path::Path::new(&shell).exists() {
        Some(shell)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_non_empty_pair() {
        let (shell, flag) = shell_command();
        assert!(!shell.is_empty());
        assert!(!flag.is_empty());
    }

    #[test]
    fn user_login_shell_respects_env_var() {
        // Points at a real binary on every Unix system.
        let saved = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", "/bin/sh");

        let shell = user_login_shell();
        assert_eq!(shell.as_deref(), Some("/bin/sh"));

        match saved {
            Some(v) => std::env::set_var("SHELL", v),
            None => std::env::remove_var("SHELL"),
        }
    }

    #[test]
    fn user_login_shell_ignores_nonexistent_path() {
        let saved = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", "/no/such/shell");

        assert_eq!(user_login_shell(), None);

        match saved {
            Some(v) => std::env::set_var("SHELL", v),
            None => std::env::remove_var("SHELL"),
        }
    }

    #[test]
    fn user_login_shell_ignores_empty_value() {
        let saved = std::env::var("SHELL").ok();
        std::env::set_var("SHELL", "");

        assert_eq!(user_login_shell(), None);

        match saved {
            Some(v) => std::env::set_var("SHELL", v),
            None => std::env::remove_var("SHELL"),
        }
    }
}
