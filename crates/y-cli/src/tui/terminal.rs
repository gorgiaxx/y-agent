//! Terminal-host capability detection used by key and clipboard fallbacks.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalHost {
    Baseline,
    Tmux,
    Ssh,
    TmuxOverSsh,
    Wsl,
    Dumb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCapabilities {
    pub host: TerminalHost,
    features: u8,
}

impl TerminalCapabilities {
    const BRACKETED_PASTE: u8 = 1 << 0;
    const NATIVE_IMAGE_CLIPBOARD: u8 = 1 << 1;
    const OSC52_COPY: u8 = 1 << 2;
    const FUNCTION_KEY_FALLBACKS: u8 = 1 << 3;

    pub fn detect() -> Self {
        Self::from_environment(
            std::env::var("TERM").ok().as_deref(),
            std::env::var_os("TMUX").is_some(),
            std::env::var_os("SSH_CONNECTION").is_some()
                || std::env::var_os("SSH_CLIENT").is_some()
                || std::env::var_os("SSH_TTY").is_some(),
            std::env::var_os("WSL_DISTRO_NAME").is_some()
                || std::env::var_os("WSL_INTEROP").is_some(),
        )
    }

    fn from_environment(term: Option<&str>, tmux: bool, ssh: bool, wsl: bool) -> Self {
        if term.is_some_and(|value| value.eq_ignore_ascii_case("dumb")) {
            return Self::for_host(TerminalHost::Dumb);
        }
        let host = match (tmux, ssh, wsl) {
            (true, true, _) => TerminalHost::TmuxOverSsh,
            (true, false, _) => TerminalHost::Tmux,
            (false, true, _) => TerminalHost::Ssh,
            (false, false, true) => TerminalHost::Wsl,
            (false, false, false) => TerminalHost::Baseline,
        };
        Self::for_host(host)
    }

    pub fn for_host(host: TerminalHost) -> Self {
        let features = match host {
            TerminalHost::Dumb => Self::FUNCTION_KEY_FALLBACKS,
            TerminalHost::Tmux | TerminalHost::Ssh | TerminalHost::TmuxOverSsh => {
                Self::BRACKETED_PASTE | Self::OSC52_COPY | Self::FUNCTION_KEY_FALLBACKS
            }
            TerminalHost::Baseline | TerminalHost::Wsl => {
                Self::BRACKETED_PASTE | Self::NATIVE_IMAGE_CLIPBOARD
            }
        };
        Self { host, features }
    }

    pub fn supports_bracketed_paste(self) -> bool {
        self.features & Self::BRACKETED_PASTE != 0
    }

    pub fn supports_native_image_clipboard(self) -> bool {
        self.features & Self::NATIVE_IMAGE_CLIPBOARD != 0
    }

    pub fn supports_osc52_copy(self) -> bool {
        self.features & Self::OSC52_COPY != 0
    }

    pub fn needs_function_key_fallbacks(self) -> bool {
        self.features & Self::FUNCTION_KEY_FALLBACKS != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_matrix_covers_baseline_tmux_and_ssh_hosts() {
        let baseline =
            TerminalCapabilities::from_environment(Some("xterm-256color"), false, false, false);
        let tmux =
            TerminalCapabilities::from_environment(Some("screen-256color"), true, false, false);
        let ssh =
            TerminalCapabilities::from_environment(Some("xterm-256color"), false, true, false);
        let nested = TerminalCapabilities::from_environment(Some("screen"), true, true, false);

        assert_eq!(baseline.host, TerminalHost::Baseline);
        assert!(baseline.supports_native_image_clipboard());
        assert_eq!(tmux.host, TerminalHost::Tmux);
        assert!(tmux.supports_osc52_copy());
        assert_eq!(ssh.host, TerminalHost::Ssh);
        assert!(ssh.needs_function_key_fallbacks());
        assert_eq!(nested.host, TerminalHost::TmuxOverSsh);
    }

    #[test]
    fn dumb_terminal_disables_escape_sequence_features() {
        let caps = TerminalCapabilities::from_environment(Some("dumb"), false, false, false);
        assert_eq!(caps.host, TerminalHost::Dumb);
        assert!(!caps.supports_bracketed_paste());
        assert!(!caps.supports_osc52_copy());
        assert!(caps.needs_function_key_fallbacks());
    }
}
