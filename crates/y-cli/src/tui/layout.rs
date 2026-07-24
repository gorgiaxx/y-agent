//! Layout engine: computes panel rectangles from terminal size and state.
//!
//! The TUI uses a command-first, single-column layout:
//! ```text
//! ┌──────────────────────────────────────────┐
//! │          Conversation (full width)       │
//! ├──────────────────────────────────────────┤
//! │          Status Bar (1 line)             │
//! ├──────────────────────────────────────────┤
//! │          Input Area (1-6 lines)          │
//! └──────────────────────────────────────────┘
//! ```

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Minimum terminal dimensions before showing "too small" warning.
pub const MIN_COLS: u16 = 60;
pub const MIN_ROWS: u16 = 15;

/// Maximum input area height as percentage of terminal height.
const INPUT_MAX_PERCENT: u16 = 30;

/// Computed layout areas for a single frame.
#[derive(Debug, Clone)]
pub struct LayoutChunks {
    /// Chat message panel.
    pub chat: Rect,
    /// Status bar (1-line).
    pub status_bar: Rect,
    /// Input area.
    pub input: Rect,
}

/// Check if terminal is too small for the TUI.
pub fn is_terminal_too_small(cols: u16, rows: u16) -> bool {
    cols < MIN_COLS || rows < MIN_ROWS
}

/// Compute the layout chunks for one frame.
///
/// `input_lines` is the current height of the input area (1-6).
pub fn compute_layout(area: Rect, input_lines: u16) -> LayoutChunks {
    // Clamp input height: 3..=8 (content 1-6 + 2 for borders),
    // and at most INPUT_MAX_PERCENT of terminal.
    let max_input = area.height * INPUT_MAX_PERCENT / 100;
    let input_height = input_lines.clamp(3, 8).min(max_input).max(3);
    let status_height = 1u16;

    // Vertical split: chat | status_bar | input.
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),                // chat (fills remaining)
            Constraint::Length(status_height), // status bar
            Constraint::Length(input_height),  // input area
        ])
        .split(area);

    LayoutChunks {
        chat: v_chunks[0],
        status_bar: v_chunks[1],
        input: v_chunks[2],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    // T-TUI-02-01: The conversation surface always uses the full width.
    #[test]
    fn test_layout_uses_full_width_without_sidebar() {
        let area = rect(120, 30);
        let layout = compute_layout(area, 1);
        assert_eq!(layout.chat.width, 120);
    }

    // T-TUI-02-02: Layout shows "too small" when terminal < 60x15.
    #[test]
    fn test_terminal_too_small() {
        assert!(is_terminal_too_small(59, 30));
        assert!(is_terminal_too_small(80, 14));
        assert!(is_terminal_too_small(50, 10));
        assert!(!is_terminal_too_small(60, 15));
        assert!(!is_terminal_too_small(120, 40));
    }

    // T-TUI-02-03: Input area height scales with content (1-6 content + 2 borders).
    #[test]
    fn test_input_height_clamped() {
        let area = rect(120, 40);

        let layout_3 = compute_layout(area, 3);
        assert_eq!(layout_3.input.height, 3);

        let layout_5 = compute_layout(area, 5);
        assert_eq!(layout_5.input.height, 5);

        let layout_8 = compute_layout(area, 8);
        assert_eq!(layout_8.input.height, 8);

        // Requests > 8 are clamped.
        let layout_12 = compute_layout(area, 12);
        assert_eq!(layout_12.input.height, 8);

        // Requests of 0 or 1 are clamped to 3 (minimum with borders).
        let layout_0 = compute_layout(area, 0);
        assert_eq!(layout_0.input.height, 3);
    }

    #[test]
    fn test_status_bar_is_one_line() {
        let area = rect(120, 40);
        let layout = compute_layout(area, 1);
        assert_eq!(layout.status_bar.height, 1);
    }
}
