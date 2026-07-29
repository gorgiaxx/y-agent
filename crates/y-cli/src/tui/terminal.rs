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
    /// Kitty keyboard-protocol enhancement so modifier-aware keys (Shift+Enter,
    /// Alt+arrows, ...) carry their modifiers instead of collapsing to the
    /// base key. Only requested for baseline hosts; nested terminals (tmux,
    /// ssh) do not reliably forward the CSI > 1 u handshake.
    const KEYBOARD_ENHANCEMENT: u8 = 1 << 4;

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
                Self::BRACKETED_PASTE | Self::NATIVE_IMAGE_CLIPBOARD | Self::KEYBOARD_ENHANCEMENT
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

    /// Whether the terminal should be asked for the Kitty keyboard-protocol
    /// enhancement. When enabled, keys like Shift+Enter and Alt+arrows arrive
    /// with their real modifiers instead of an unmodified Enter/arrow.
    pub fn supports_keyboard_enhancement(self) -> bool {
        self.features & Self::KEYBOARD_ENHANCEMENT != 0
    }
}

/// Whether the terminal is expected to ship Nerd Font glyphs (powerline
/// separators like `\u{E0B1}`). Mirrors pi-powerline-footer's detection: an
/// explicit `Y_AGENT_NERD_FONTS` override wins, then well-known
/// Nerd-Font-capable terminal programs; Apple Terminal and unknown hosts
/// fall back to ASCII separators.
pub fn nerd_font_available() -> bool {
    nerd_font_from_environment(
        std::env::var("Y_AGENT_NERD_FONTS").ok().as_deref(),
        std::env::var("TERM_PROGRAM").ok().as_deref(),
    )
}

fn nerd_font_from_environment(override_flag: Option<&str>, term_program: Option<&str>) -> bool {
    if let Some(flag) = override_flag {
        return !matches!(flag, "0" | "false" | "no");
    }
    term_program.is_some_and(|program| {
        matches!(
            program.to_ascii_lowercase().as_str(),
            "iterm.app" | "wezterm" | "kitty" | "ghostty" | "alacritty"
        )
    })
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
        assert!(baseline.supports_keyboard_enhancement());
        assert_eq!(tmux.host, TerminalHost::Tmux);
        assert!(tmux.supports_osc52_copy());
        assert!(
            !tmux.supports_keyboard_enhancement(),
            "tmux must not enable kitty kb"
        );
        assert_eq!(ssh.host, TerminalHost::Ssh);
        assert!(ssh.needs_function_key_fallbacks());
        assert!(
            !ssh.supports_keyboard_enhancement(),
            "ssh must not enable kitty kb"
        );
        assert_eq!(nested.host, TerminalHost::TmuxOverSsh);
    }

    #[test]
    fn dumb_terminal_disables_escape_sequence_features() {
        let caps = TerminalCapabilities::from_environment(Some("dumb"), false, false, false);
        assert_eq!(caps.host, TerminalHost::Dumb);
        assert!(!caps.supports_bracketed_paste());
        assert!(!caps.supports_osc52_copy());
        assert!(!caps.supports_keyboard_enhancement());
        assert!(caps.needs_function_key_fallbacks());
    }

    #[test]
    fn nerd_font_detection_matrix() {
        assert!(nerd_font_from_environment(None, Some("iTerm.app")));
        assert!(nerd_font_from_environment(None, Some("WezTerm")));
        assert!(nerd_font_from_environment(None, Some("ghostty")));
        assert!(!nerd_font_from_environment(None, Some("Apple_Terminal")));
        assert!(!nerd_font_from_environment(None, Some("vscode")));
        assert!(!nerd_font_from_environment(None, None));
        // Explicit override wins in both directions.
        assert!(nerd_font_from_environment(
            Some("1"),
            Some("Apple_Terminal")
        ));
        assert!(!nerd_font_from_environment(Some("0"), Some("iTerm.app")));
    }
}
