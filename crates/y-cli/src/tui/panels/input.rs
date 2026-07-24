//! Input area panel renderer.
//!
//! Multi-line input area using `tui-textarea` for editing support.
//! Auto-expands height based on content (1-6 lines).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;
use tui_textarea::TextArea;

use crate::tui::state::PanelFocus;
use crate::tui::theme::Theme;

/// Render the input area into the given area.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    focus: PanelFocus,
    is_streaming: bool,
    is_cancelling: bool,
    textarea: &TextArea<'_>,
    t: &Theme,
) {
    let is_focused = focus == PanelFocus::Input;

    let border_style = if is_focused {
        Style::default().fg(t.input_border_focused())
    } else {
        Style::default().fg(t.input_border_unfocused())
    };

    let title = input_title(focus, is_streaming, is_cancelling);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title)
        .title_style(
            Style::default()
                .fg(t.input_title())
                .add_modifier(Modifier::BOLD),
        );

    let mut ta = textarea.clone();
    ta.set_block(block);
    ta.set_cursor_line_style(Style::default());

    if is_focused {
        ta.set_cursor_style(
            Style::default()
                .fg(t.cursor_fg())
                .bg(t.cursor_bg())
                .add_modifier(Modifier::BOLD),
        );
    } else {
        ta.set_cursor_style(Style::default().fg(t.cursor_unfocused()));
    }

    frame.render_widget(&ta, area);
}

fn input_title(focus: PanelFocus, is_streaming: bool, is_cancelling: bool) -> &'static str {
    if focus != PanelFocus::Input {
        return " Message ";
    }
    if is_cancelling {
        return " Follow-up  Cancelling... ";
    }
    if is_streaming {
        return " Follow-up  Enter queue  Esc cancel ";
    }
    " Message  / commands  Enter send "
}

/// Calculate the desired input area height based on content.
///
/// Returns content lines + 2 (for top/bottom borders), clamped so content
/// is between 1 and 6 lines. The border accounts for `Borders::ALL`.
pub fn input_height(textarea: &TextArea<'_>) -> u16 {
    let line_count = textarea.lines().len().max(1);
    let content = u16::try_from(line_count).unwrap_or(1).clamp(1, 6);
    content + 2 // +2 for top and bottom border
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_height_single_line() {
        let textarea = TextArea::default();
        assert_eq!(input_height(&textarea), 3); // 1 content + 2 borders
    }

    #[test]
    fn test_input_height_multi_line() {
        let lines = vec!["line 1", "line 2", "line 3"];
        let textarea = TextArea::new(lines.into_iter().map(String::from).collect());
        assert_eq!(input_height(&textarea), 5); // 3 content + 2 borders
    }

    #[test]
    fn test_input_height_capped_at_six() {
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let textarea = TextArea::new(lines);
        assert_eq!(input_height(&textarea), 8); // 6 content + 2 borders
    }

    #[test]
    fn test_input_title_explains_follow_up_and_cancel_during_streaming() {
        assert_eq!(
            input_title(PanelFocus::Input, true, false),
            " Follow-up  Enter queue  Esc cancel "
        );
    }

    #[test]
    fn test_input_title_reports_pending_cancellation() {
        assert_eq!(
            input_title(PanelFocus::Input, true, true),
            " Follow-up  Cancelling... "
        );
    }
}
