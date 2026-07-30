//! Layout engine: computes panel rectangles from terminal size and state.
//!
//! The TUI uses a command-first, single-column layout:
//! ```text
//! ┌──────────────────────────────────────────┐
//! │          Conversation (full width)       │
//! ├──────────────────────────────────────────┤
//! │          Active TODO queue                │
//! ├──────────────────────────────────────────┤
//! │          Input Area (1-6 lines)          │
//! ├──────────────────────────────────────────┤
//! │          Status Bar (1 line)             │
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
    /// Active-run TODO queue immediately above the composer.
    pub todo: Rect,
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
pub fn compute_layout(area: Rect, input_lines: u16, todo_count: usize) -> LayoutChunks {
    // Clamp input height: 3..=8 (content 1-6 + 2 for borders),
    // and at most INPUT_MAX_PERCENT of terminal.
    let max_input = area.height * INPUT_MAX_PERCENT / 100;
    let input_height = input_lines.clamp(3, 8).min(max_input).max(3);
    let status_height = 1u16;
    let todo_height = todo_panel_height(todo_count);

    // Vertical split: chat | TODO queue | input | status bar (bottom row).
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),                // chat (fills remaining)
            Constraint::Length(todo_height),   // active-run TODO queue
            Constraint::Length(input_height),  // input area
            Constraint::Length(status_height), // status bar
        ])
        .split(area);

    LayoutChunks {
        chat: v_chunks[0],
        todo: v_chunks[1],
        input: v_chunks[2],
        status_bar: v_chunks[3],
    }
}

/// Inline TODO rows are bounded so the composer cannot crowd out the transcript.
pub const fn todo_panel_height(todo_count: usize) -> u16 {
    if todo_count == 0 {
        return 0;
    }
    let visible = if todo_count > 4 { 4 } else { todo_count } as u16;
    let overflow = if todo_count > 4 { 1 } else { 0 };
    1 + visible + overflow
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
        let layout = compute_layout(area, 1, 0);
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

        let layout_3 = compute_layout(area, 3, 0);
        assert_eq!(layout_3.input.height, 3);

        let layout_5 = compute_layout(area, 5, 0);
        assert_eq!(layout_5.input.height, 5);

        let layout_8 = compute_layout(area, 8, 0);
        assert_eq!(layout_8.input.height, 8);

        // Requests > 8 are clamped.
        let layout_12 = compute_layout(area, 12, 0);
        assert_eq!(layout_12.input.height, 8);

        // Requests of 0 or 1 are clamped to 3 (minimum with borders).
        let layout_0 = compute_layout(area, 0, 0);
        assert_eq!(layout_0.input.height, 3);
    }

    #[test]
    fn test_status_bar_is_one_line() {
        let area = rect(120, 40);
        let layout = compute_layout(area, 1, 0);
        assert_eq!(layout.status_bar.height, 1);
    }

    // T-TUI-02-05: The status bar sits at the very bottom, below the input.
    #[test]
    fn test_status_bar_below_input() {
        let area = rect(120, 40);
        let layout = compute_layout(area, 3, 0);
        assert!(
            layout.status_bar.y >= layout.input.y + layout.input.height,
            "status bar must be placed below the input area"
        );
        assert_eq!(
            layout.status_bar.y + layout.status_bar.height,
            area.y + area.height,
            "status bar must occupy the terminal's bottom row"
        );
    }

    #[test]
    fn test_todo_panel_is_above_input_and_hidden_when_empty() {
        let area = rect(120, 40);
        let empty = compute_layout(area, 3, 0);
        assert_eq!(empty.todo.height, 0);

        let populated = compute_layout(area, 3, 2);
        assert!(populated.todo.height > 0);
        assert_eq!(
            populated.todo.y + populated.todo.height,
            populated.input.y,
            "TODO panel must sit immediately above the composer"
        );
    }
}
