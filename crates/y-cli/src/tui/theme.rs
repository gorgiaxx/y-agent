//! Terminal-aware color theme for TUI rendering.
//!
//! macOS Terminal.app does not support truecolor (24-bit RGB) escape sequences.
//! When crossterm emits `\x1b[38;2;R;G;Bm`, Terminal.app either ignores it or
//! renders an incorrect color, causing both the "wrong colors" bug and the
//! "partial selection" visual artifact (inconsistent backgrounds between Spans
//! that have explicit bg and those that fall back to terminal default).
//!
//! This module detects terminal capabilities via `COLORTERM` and `TERM` env vars
//! and selects the appropriate color palette at startup:
//!
//! | Terminal              | `COLORTERM`   | Palette used |
//! |-----------------------|---------------|-------------|
//! | iTerm2, Alacritty... | `truecolor`   | RGB (rich)  |
//! | macOS Terminal.app    | _(empty)_     | 256-color   |
//! | xterm, screen        | other / unset | 256-color   |
//!
//! Every semantic color has both an RGB value and a 256-color `Indexed` fallback.
//! The 256-color values are chosen from the 6x6x6 color cube (indices 16-231)
//! and the 24-step grayscale ramp (indices 232-255) to closely approximate the
//! RGB originals.

use ratatui::style::Color;

// ---------------------------------------------------------------------------
// Terminal capability detection
// ---------------------------------------------------------------------------

/// Whether the terminal advertises truecolor (24-bit RGB) support.
///
/// Detection strategy (matches what crossterm/VTE apps commonly check):
/// 1. `COLORTERM=truecolor` or `COLORTERM=24bit` -> truecolor
/// 2. Otherwise -> assume 256-color only
///
/// Terminals that set `COLORTERM=truecolor`: iTerm2, Alacritty, Kitty,
/// `WezTerm`, Ghostty, Windows Terminal, foot, etc.
///
/// macOS Terminal.app does **not** set `COLORTERM` at all.
fn terminal_supports_truecolor() -> bool {
    std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit")
}

// ---------------------------------------------------------------------------
// Theme struct
// ---------------------------------------------------------------------------

/// Color theme that adapts to terminal capabilities.
///
/// Provides semantic colors for every visual element in the TUI. On truecolor
/// terminals the richer RGB palette is used; on limited terminals (macOS
/// Terminal.app) the 256-color fallback indices are used instead.
///
/// Use `Theme::default()` to obtain the auto-detected singleton.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Whether the connected terminal supports truecolor.
    truecolor: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            truecolor: terminal_supports_truecolor(),
        }
    }
}

impl Theme {
    // -----------------------------------------------------------------------
    // Helper: pick RGB or Indexed
    // -----------------------------------------------------------------------

    /// Return the RGB color on truecolor terminals, or the 256-color index
    /// otherwise.
    const fn color(&self, rgb: Color, idx: Color) -> Color {
        match (self.truecolor, rgb, idx) {
            (true, rgb, _) => rgb,
            (false, _, idx) => idx,
        }
    }

    // -----------------------------------------------------------------------
    // Backgrounds
    // -----------------------------------------------------------------------

    /// Dark panel background.
    pub fn panel_bg(&self) -> Color {
        // RGB(22,22,30) -> grayscale index 234 (very dark blue-gray)
        self.color(Color::Rgb(22, 22, 30), Color::Indexed(234))
    }

    /// Code block background.
    pub fn code_bg(&self) -> Color {
        // RGB(40,42,54) -> index 235 (Dracula-ish dark)
        self.color(Color::Rgb(40, 42, 54), Color::Indexed(235))
    }

    // -----------------------------------------------------------------------
    // Borders
    // -----------------------------------------------------------------------

    /// Focused panel border.
    pub fn border_focused(&self) -> Color {
        // RGB(120,180,255) -> index 75 (blue, 6x6x6 cube)
        self.color(Color::Rgb(120, 180, 255), Color::Indexed(75))
    }

    /// Unfocused panel border.
    pub fn border_unfocused(&self) -> Color {
        // RGB(50,50,65) -> grayscale index 239
        self.color(Color::Rgb(50, 50, 65), Color::Indexed(239))
    }

    // -----------------------------------------------------------------------
    // Text
    // -----------------------------------------------------------------------

    /// Panel title text.
    pub fn title(&self) -> Color {
        // RGB(180,180,200) -> index 252 (light gray)
        self.color(Color::Rgb(180, 180, 200), Color::Indexed(252))
    }

    /// Primary content text.
    pub fn text(&self) -> Color {
        // RGB(220,220,230) -> index 254 (near-white)
        self.color(Color::Rgb(220, 220, 230), Color::Indexed(254))
    }

    /// Muted / secondary text.
    pub fn muted(&self) -> Color {
        // RGB(100,100,120) -> index 245 (mid gray)
        self.color(Color::Rgb(100, 100, 120), Color::Indexed(245))
    }

    /// Empty-state placeholder text.
    pub fn empty(&self) -> Color {
        // RGB(80,80,100) -> index 245 (slightly dimmer than muted)
        self.color(Color::Rgb(80, 80, 100), Color::Indexed(245))
    }

    // -----------------------------------------------------------------------
    // Role accents
    // -----------------------------------------------------------------------

    /// User role accent (green).
    pub fn user_accent(&self) -> Color {
        // RGB(130,220,130) -> index 114 (bright green)
        self.color(Color::Rgb(130, 220, 130), Color::Indexed(114))
    }

    /// Assistant role accent (blue).
    pub fn assistant_accent(&self) -> Color {
        // RGB(120,180,255) -> index 75 (blue)
        self.color(Color::Rgb(120, 180, 255), Color::Indexed(75))
    }

    /// System role accent (yellow).
    pub fn system_accent(&self) -> Color {
        // RGB(220,200,100) -> index 179 (yellow)
        self.color(Color::Rgb(220, 200, 100), Color::Indexed(179))
    }

    /// Tool role accent (purple).
    pub fn tool_accent(&self) -> Color {
        // RGB(200,140,255) -> index 177 (light magenta)
        self.color(Color::Rgb(200, 140, 255), Color::Indexed(177))
    }

    // -----------------------------------------------------------------------
    // List / selection
    // -----------------------------------------------------------------------

    /// Selected item highlight color.
    pub fn selected(&self) -> Color {
        self.color(Color::Rgb(120, 180, 255), Color::Indexed(75))
    }

    /// Active / current indicator color.
    pub fn active(&self) -> Color {
        self.color(Color::Rgb(130, 220, 130), Color::Indexed(114))
    }

    /// Normal (non-selected) item text.
    pub fn normal(&self) -> Color {
        self.color(Color::Rgb(180, 180, 200), Color::Indexed(252))
    }

    /// "New Session" action text.
    pub fn new_session(&self) -> Color {
        // RGB(100,160,255) -> index 69 (blue)
        self.color(Color::Rgb(100, 160, 255), Color::Indexed(69))
    }

    // -----------------------------------------------------------------------
    // Status
    // -----------------------------------------------------------------------

    /// Success / done status.
    pub fn success(&self) -> Color {
        // RGB(100,200,120) -> index 114 (green)
        self.color(Color::Rgb(100, 200, 120), Color::Indexed(114))
    }

    /// Error status.
    pub fn error(&self) -> Color {
        // RGB(255,100,100) -> index 167 (bright red)
        self.color(Color::Rgb(255, 100, 100), Color::Indexed(167))
    }

    /// Warning / running / streaming status.
    pub fn warning(&self) -> Color {
        // RGB(255,200,60) -> index 179 (yellow)
        self.color(Color::Rgb(255, 200, 60), Color::Indexed(179))
    }

    /// Streaming indicator dot.
    pub fn streaming_dot(&self) -> Color {
        self.warning()
    }

    // -----------------------------------------------------------------------
    // Cards (Thinking, ToolCall)
    // -----------------------------------------------------------------------

    /// Thinking card accent (purple).
    pub fn think_accent(&self) -> Color {
        // RGB(167,139,250) -> index 135 (purple)
        self.color(Color::Rgb(167, 139, 250), Color::Indexed(135))
    }

    /// Thinking card content text.
    pub fn think_text(&self) -> Color {
        // RGB(160,150,200) -> index 183 (light lavender)
        self.color(Color::Rgb(160, 150, 200), Color::Indexed(183))
    }

    /// Tool call card accent (cyan-blue).
    pub fn tool_card_accent(&self) -> Color {
        // RGB(0,166,255) -> index 39 (bright cyan)
        self.color(Color::Rgb(0, 166, 255), Color::Indexed(39))
    }

    /// Tool call card content text.
    pub fn tool_card_text(&self) -> Color {
        // RGB(140,170,200) -> index 152 (steel blue)
        self.color(Color::Rgb(140, 170, 200), Color::Indexed(152))
    }

    // -----------------------------------------------------------------------
    // Markdown
    // -----------------------------------------------------------------------

    /// Blockquote accent / border.
    pub fn blockquote(&self) -> Color {
        // RGB(100,120,160) -> index 67 (dark steel blue)
        self.color(Color::Rgb(100, 120, 160), Color::Indexed(67))
    }

    /// Horizontal rule color.
    pub fn hr(&self) -> Color {
        // RGB(60,60,80) -> index 241 (dark gray)
        self.color(Color::Rgb(60, 60, 80), Color::Indexed(241))
    }

    /// Welcome screen accent.
    pub fn welcome(&self) -> Color {
        // RGB(100,120,180) -> index 68 (slate blue)
        self.color(Color::Rgb(100, 120, 180), Color::Indexed(68))
    }

    /// Inline code text color.
    pub fn code_fg(&self) -> Color {
        // RGB(200,220,255) -> index 189 (light blue-white)
        self.color(Color::Rgb(200, 220, 255), Color::Indexed(189))
    }

    /// Code block content text color.
    pub fn code_block_fg(&self) -> Color {
        // RGB(180,200,220) -> index 152 (steel blue)
        self.color(Color::Rgb(180, 200, 220), Color::Indexed(152))
    }

    // -----------------------------------------------------------------------
    // Status bar
    // -----------------------------------------------------------------------

    /// Status bar background (same as `panel_bg`).
    pub fn status_bg(&self) -> Color {
        self.panel_bg()
    }

    /// Model name text.
    pub fn status_model(&self) -> Color {
        // RGB(180,140,255) -> index 183 (light purple)
        self.color(Color::Rgb(180, 140, 255), Color::Indexed(183))
    }

    /// Token ratio text.
    pub fn status_token_ratio(&self) -> Color {
        // RGB(150,150,170) -> index 252 (light gray)
        self.color(Color::Rgb(150, 150, 170), Color::Indexed(252))
    }

    /// Context bar track (empty portion).
    pub fn status_bar_track(&self) -> Color {
        // RGB(45,45,60) -> index 239 (dark gray)
        self.color(Color::Rgb(45, 45, 60), Color::Indexed(239))
    }

    /// Context bar fill (normal, < 80%).
    pub fn status_bar_normal(&self) -> Color {
        // RGB(100,140,255) -> index 69 (blue)
        self.color(Color::Rgb(100, 140, 255), Color::Indexed(69))
    }

    /// Context bar fill (warning, >= 80%).
    pub fn status_bar_warn(&self) -> Color {
        // RGB(240,192,80) -> index 179 (yellow)
        self.color(Color::Rgb(240, 192, 80), Color::Indexed(179))
    }

    /// Separator between status items.
    pub fn status_sep(&self) -> Color {
        self.hr()
    }

    /// Cost text.
    pub fn status_cost(&self) -> Color {
        // RGB(130,130,150) -> index 245 (gray)
        self.color(Color::Rgb(130, 130, 150), Color::Indexed(245))
    }

    /// Version text.
    pub fn status_version(&self) -> Color {
        // RGB(80,80,100) -> index 245 (dark gray)
        self.color(Color::Rgb(80, 80, 100), Color::Indexed(245))
    }

    // -----------------------------------------------------------------------
    // Input area
    // -----------------------------------------------------------------------

    /// Input area focused border.
    pub fn input_border_focused(&self) -> Color {
        // Cyan -> index 81 (bright cyan on 256-color)
        self.color(Color::Cyan, Color::Indexed(81))
    }

    /// Input area unfocused border.
    pub fn input_border_unfocused(&self) -> Color {
        // DarkGray -> index 245
        self.color(Color::DarkGray, Color::Indexed(245))
    }

    /// Input area title text.
    pub fn input_title(&self) -> Color {
        self.color(Color::White, Color::Indexed(255))
    }

    /// Cursor style foreground (when focused).
    pub fn cursor_fg(&self) -> Color {
        self.color(Color::Black, Color::Indexed(16))
    }

    /// Cursor style background (when focused).
    pub fn cursor_bg(&self) -> Color {
        self.color(Color::White, Color::Indexed(255))
    }

    /// Unfocused cursor.
    pub fn cursor_unfocused(&self) -> Color {
        self.color(Color::DarkGray, Color::Indexed(245))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_default_respects_env() {
        // We cannot reliably test the env var in parallel tests,
        // but we can verify the struct works.
        let theme = Theme::default();
        // Just ensure it doesn't panic and returns a valid color.
        let _ = theme.panel_bg();
        let _ = theme.border_focused();
        let _ = theme.text();
    }

    #[test]
    fn test_truecolor_returns_rgb() {
        let theme = Theme { truecolor: true };
        assert!(matches!(theme.panel_bg(), Color::Rgb(_, _, _)));
        assert!(matches!(theme.border_focused(), Color::Rgb(_, _, _)));
        assert!(matches!(theme.text(), Color::Rgb(_, _, _)));
    }

    #[test]
    fn test_no_truecolor_returns_indexed() {
        let theme = Theme { truecolor: false };
        assert!(matches!(theme.panel_bg(), Color::Indexed(_)));
        assert!(matches!(theme.border_focused(), Color::Indexed(_)));
        assert!(matches!(theme.text(), Color::Indexed(_)));
    }

    #[test]
    fn test_all_colors_returned() {
        let theme = Theme::default();
        // Exercise every method to ensure no panic.
        let _ = theme.panel_bg();
        let _ = theme.code_bg();
        let _ = theme.border_focused();
        let _ = theme.border_unfocused();
        let _ = theme.title();
        let _ = theme.text();
        let _ = theme.muted();
        let _ = theme.empty();
        let _ = theme.user_accent();
        let _ = theme.assistant_accent();
        let _ = theme.system_accent();
        let _ = theme.tool_accent();
        let _ = theme.selected();
        let _ = theme.active();
        let _ = theme.normal();
        let _ = theme.new_session();
        let _ = theme.success();
        let _ = theme.error();
        let _ = theme.warning();
        let _ = theme.streaming_dot();
        let _ = theme.think_accent();
        let _ = theme.think_text();
        let _ = theme.tool_card_accent();
        let _ = theme.tool_card_text();
        let _ = theme.blockquote();
        let _ = theme.hr();
        let _ = theme.welcome();
        let _ = theme.code_fg();
        let _ = theme.code_block_fg();
        let _ = theme.status_bg();
        let _ = theme.status_model();
        let _ = theme.status_token_ratio();
        let _ = theme.status_bar_track();
        let _ = theme.status_bar_normal();
        let _ = theme.status_bar_warn();
        let _ = theme.status_sep();
        let _ = theme.status_cost();
        let _ = theme.status_version();
        let _ = theme.input_border_focused();
        let _ = theme.input_border_unfocused();
        let _ = theme.input_title();
        let _ = theme.cursor_fg();
        let _ = theme.cursor_bg();
        let _ = theme.cursor_unfocused();
    }
}
