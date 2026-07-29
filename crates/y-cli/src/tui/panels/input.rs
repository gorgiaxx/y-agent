//! Input area panel renderer.
//!
//! Multi-line input area using `tui-textarea` for editing support.
//! Auto-expands height based on content (1-6 lines).

use std::cell::Cell;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;
use tui_textarea::TextArea;

use crate::tui::state::{InteractionMode, PanelFocus};
use crate::tui::theme::Theme;

/// Style-relevant render state. Styles are only re-applied to the textarea
/// when this changes, so frames no longer clone the whole `TextArea` just to
/// attach a block and cursor style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StyleState {
    focused: bool,
    mode: InteractionMode,
    streaming: bool,
    cancelling: bool,
    /// Follow-up queue depth, shown in the streaming title.
    follow_up_count: usize,
}

thread_local! {
    /// Last style state applied to the composer textarea by [`render`].
    static LAST_STYLE_STATE: Cell<Option<StyleState>> = const { Cell::new(None) };
}

/// Render the input area into the given area.
///
/// Applies block/cursor styles to `textarea` only when the style state
/// changed since the last frame (or the textarea was replaced), then renders
/// it by reference — `tui-textarea` implements `Widget for &TextArea`.
///
/// `follow_up_count` is the current follow-up queue depth; while streaming it
/// is shown in the title (`Follow-up (N)`).
pub fn render(
    frame: &mut Frame,
    area: Rect,
    focus: PanelFocus,
    mode: InteractionMode,
    is_streaming: bool,
    is_cancelling: bool,
    follow_up_count: usize,
    textarea: &mut TextArea<'_>,
    t: &Theme,
) {
    let is_focused = focus == PanelFocus::Input;
    let style_state = StyleState {
        focused: is_focused,
        mode,
        streaming: is_streaming,
        cancelling: is_cancelling,
        follow_up_count,
    };

    // A freshly created `TextArea` carries tui-textarea's default cursor
    // line style (underlined), so it fails this check and gets restyled even
    // if the cached state happens to match.
    let styles_applied = LAST_STYLE_STATE.with(Cell::get) == Some(style_state)
        && textarea.cursor_line_style() == Style::default();

    if !styles_applied {
        let border_style = if is_focused {
            Style::default().fg(t.input_border_focused())
        } else {
            Style::default().fg(t.input_border_unfocused())
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(input_title(
                focus,
                mode,
                is_streaming,
                is_cancelling,
                follow_up_count,
            ))
            .title_style(
                Style::default()
                    .fg(t.input_title())
                    .add_modifier(Modifier::BOLD),
            );

        textarea.set_block(block);
        textarea.set_cursor_line_style(Style::default());
        textarea.set_cursor_style(if is_focused {
            Style::default()
                .fg(t.cursor_fg())
                .bg(t.cursor_bg())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.cursor_unfocused())
        });

        LAST_STYLE_STATE.with(|last| last.set(Some(style_state)));
    }

    frame.render_widget(&*textarea, area);
}

fn input_title(
    focus: PanelFocus,
    mode: InteractionMode,
    is_streaming: bool,
    is_cancelling: bool,
    follow_up_count: usize,
) -> String {
    if focus != PanelFocus::Input {
        return " Message ".to_string();
    }
    if mode == InteractionMode::Shell {
        if is_cancelling {
            return " Shell  Cancelling... ".to_string();
        }
        if is_streaming {
            return " Shell  Running...  Esc cancel ".to_string();
        }
        return " Shell  Enter run  Esc exit ".to_string();
    }
    if is_cancelling {
        return " Follow-up  Cancelling... ".to_string();
    }
    if is_streaming {
        return format!(" Follow-up ({follow_up_count})  Enter queue  Esc cancel ");
    }
    " Message  / commands  Enter send ".to_string()
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
            input_title(PanelFocus::Input, InteractionMode::Normal, true, false, 0),
            " Follow-up (0)  Enter queue  Esc cancel "
        );
    }

    #[test]
    fn test_input_title_shows_follow_up_queue_depth_during_streaming() {
        assert_eq!(
            input_title(PanelFocus::Input, InteractionMode::Normal, true, false, 3),
            " Follow-up (3)  Enter queue  Esc cancel "
        );
    }

    #[test]
    fn test_input_title_reports_pending_cancellation() {
        assert_eq!(
            input_title(PanelFocus::Input, InteractionMode::Normal, true, true, 2),
            " Follow-up  Cancelling... "
        );
    }

    #[test]
    fn test_input_title_unfocused_and_idle() {
        assert_eq!(
            input_title(PanelFocus::Chat, InteractionMode::Normal, true, false, 5),
            " Message "
        );
        assert_eq!(
            input_title(PanelFocus::Input, InteractionMode::Normal, false, false, 0),
            " Message  / commands  Enter send "
        );
    }

    #[test]
    fn test_input_title_identifies_shell_mode() {
        assert_eq!(
            input_title(PanelFocus::Input, InteractionMode::Shell, false, false, 0),
            " Shell  Enter run  Esc exit "
        );
        assert_eq!(
            input_title(PanelFocus::Input, InteractionMode::Shell, true, false, 0),
            " Shell  Running...  Esc cancel "
        );
    }
}
