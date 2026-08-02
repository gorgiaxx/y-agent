//! Input area panel renderer.
//!
//! Multi-line input area using `tui-textarea` for editing support.
//! Auto-expands height based on content (1-6 lines).

use std::cell::Cell;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use tui_textarea::TextArea;

use crate::tui::keys::{platform_shortcut_label, KeyAction, KeyContext, Keymap};
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
/// While the composer is empty, the usage hint is drawn inside the box as a
/// dim placeholder instead of sitting on the border as a title.
/// `follow_up_count` is the current TODO queue depth, shown in the streaming
/// placeholder (` TODO (N)`).
pub fn render(
    frame: &mut Frame,
    area: Rect,
    focus: PanelFocus,
    mode: InteractionMode,
    is_streaming: bool,
    is_cancelling: bool,
    follow_up_count: usize,
    textarea: &mut TextArea<'_>,
    keymap: &Keymap,
    t: &Theme,
) {
    let is_focused = focus == PanelFocus::Input;
    let style_state = StyleState {
        focused: is_focused,
        mode,
        streaming: is_streaming,
        cancelling: is_cancelling,
    };

    // A freshly created `TextArea` carries tui-textarea's default cursor
    // line style (underlined), so it fails this check and gets restyled even
    // if the cached state happens to match.
    let styles_applied = LAST_STYLE_STATE.with(Cell::get) == Some(style_state)
        && textarea.cursor_line_style() == Style::default();

    if !styles_applied {
        let border_style = if mode == InteractionMode::Command {
            Style::default().fg(t.warning())
        } else if is_focused {
            Style::default().fg(t.input_border_focused())
        } else {
            Style::default().fg(t.input_border_unfocused())
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);

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

    // Placeholder: the usage hint lives inside the box while the composer is
    // empty, instead of sitting on the border as a title.
    if textarea_is_empty(textarea) {
        let inner = Rect {
            x: area.x.saturating_add(1),
            y: area.y.saturating_add(1),
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        if inner.width > 0 && inner.height > 0 {
            let hint = input_hint(
                focus,
                mode,
                is_streaming,
                is_cancelling,
                follow_up_count,
                keymap,
            );
            frame.render_widget(
                Paragraph::new(hint).style(Style::default().fg(t.muted())),
                inner,
            );
        }
    }
}

/// Whether the composer holds no text at all.
fn textarea_is_empty(textarea: &TextArea<'_>) -> bool {
    textarea.lines().iter().all(std::string::String::is_empty)
}

fn input_hint(
    focus: PanelFocus,
    mode: InteractionMode,
    is_streaming: bool,
    is_cancelling: bool,
    follow_up_count: usize,
    keymap: &Keymap,
) -> String {
    if focus != PanelFocus::Input {
        return "Message".to_string();
    }
    if mode == InteractionMode::Shell {
        if is_cancelling {
            return "Shell  Cancelling...".to_string();
        }
        if is_streaming {
            return format!(
                "Shell  Running...  {}",
                input_action_hint(
                    keymap,
                    KeyContext::Streaming,
                    KeyAction::CancelStreaming,
                    "cancel"
                )
            );
        }
        return format!(
            "Shell  {}  {}",
            input_action_hint(keymap, KeyContext::Shell, KeyAction::Submit, "run"),
            input_action_hint(keymap, KeyContext::Shell, KeyAction::ReturnToNormal, "exit")
        );
    }
    if is_cancelling {
        return " TODO  Cancelling...".to_string();
    }
    if is_streaming {
        return format!(
            "TODO ({follow_up_count})  {}  {}",
            input_action_hint(
                keymap,
                KeyContext::NormalInputEmpty,
                KeyAction::Submit,
                "queue"
            ),
            input_action_hint(
                keymap,
                KeyContext::Streaming,
                KeyAction::CancelStreaming,
                "cancel"
            )
        );
    }
    format!(
        "Message  {}  / commands",
        input_action_hint(
            keymap,
            KeyContext::NormalInputEmpty,
            KeyAction::Submit,
            "send"
        )
    )
}

fn input_action_hint(
    keymap: &Keymap,
    context: KeyContext,
    action: KeyAction,
    label: &str,
) -> String {
    keymap
        .primary_shortcut_in_context(context, action)
        .map_or_else(
            || label.to_string(),
            |shortcut| format!("{} {label}", platform_shortcut_label(&shortcut)),
        )
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
    fn test_input_hint_explains_todo_queue_and_cancel_during_streaming() {
        assert_eq!(
            input_hint(
                PanelFocus::Input,
                InteractionMode::Normal,
                true,
                false,
                0,
                &Keymap::default(),
            ),
            "TODO (0)  Enter queue  Esc cancel"
        );
    }

    #[test]
    fn test_input_hint_shows_todo_queue_depth_during_streaming() {
        assert_eq!(
            input_hint(
                PanelFocus::Input,
                InteractionMode::Normal,
                true,
                false,
                3,
                &Keymap::default(),
            ),
            "TODO (3)  Enter queue  Esc cancel"
        );
    }

    #[test]
    fn test_input_hint_reports_pending_cancellation() {
        assert_eq!(
            input_hint(
                PanelFocus::Input,
                InteractionMode::Normal,
                true,
                true,
                2,
                &Keymap::default(),
            ),
            " TODO  Cancelling..."
        );
    }

    #[test]
    fn test_input_hint_unfocused_and_idle() {
        assert_eq!(
            input_hint(
                PanelFocus::Chat,
                InteractionMode::Normal,
                true,
                false,
                5,
                &Keymap::default(),
            ),
            "Message"
        );
        assert_eq!(
            input_hint(
                PanelFocus::Input,
                InteractionMode::Normal,
                false,
                false,
                0,
                &Keymap::default(),
            ),
            "Message  Enter send  / commands"
        );
    }

    #[test]
    fn test_input_hint_identifies_shell_mode() {
        assert_eq!(
            input_hint(
                PanelFocus::Input,
                InteractionMode::Shell,
                false,
                false,
                0,
                &Keymap::default(),
            ),
            "Shell  Enter run  Esc exit"
        );
        assert_eq!(
            input_hint(
                PanelFocus::Input,
                InteractionMode::Shell,
                true,
                false,
                0,
                &Keymap::default(),
            ),
            "Shell  Running...  Esc cancel"
        );
    }

    fn row_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width;
        (0..width)
            .filter_map(|x| buffer.cell((x, y)).map(ratatui::buffer::Cell::symbol))
            .collect()
    }

    // T-INPUT-PLACEHOLDER-01: the hint renders inside the box (not on the
    // border) while the composer is empty.
    #[test]
    fn test_placeholder_visible_inside_empty_composer() {
        let backend = ratatui::backend::TestBackend::new(44, 3);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut textarea = TextArea::default();
        let theme = Theme::default();
        let keymap = Keymap::default();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    PanelFocus::Input,
                    InteractionMode::Normal,
                    false,
                    false,
                    0,
                    &mut textarea,
                    &keymap,
                    &theme,
                );
            })
            .unwrap();

        // The top border row must not carry the hint text anymore.
        let top_row = row_text(&terminal, 0);
        assert!(
            !top_row.contains("Message"),
            "border must not show the hint: {top_row:?}"
        );
        // The hint appears on the first content row inside the box.
        let inner_row = row_text(&terminal, 1);
        assert!(
            inner_row.contains("Message  Enter send  / commands"),
            "placeholder must render inside the box: {inner_row:?}"
        );
    }

    // T-INPUT-PLACEHOLDER-02: typing hides the placeholder.
    #[test]
    fn test_placeholder_hidden_when_composer_has_text() {
        let backend = ratatui::backend::TestBackend::new(44, 3);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut textarea = TextArea::new(vec!["hi".to_string()]);
        let theme = Theme::default();
        let keymap = Keymap::default();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    PanelFocus::Input,
                    InteractionMode::Normal,
                    false,
                    false,
                    0,
                    &mut textarea,
                    &keymap,
                    &theme,
                );
            })
            .unwrap();

        let inner_row = row_text(&terminal, 1);
        assert!(
            !inner_row.contains("/ commands"),
            "placeholder must disappear once text is present: {inner_row:?}"
        );
        assert!(inner_row.contains("hi"));
    }
}
