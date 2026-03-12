//! Toast overlay: renders transient notifications stacked in the bottom-right.
//!
//! Toasts are non-modal: they do not capture keyboard input. Each toast has
//! a colored left border indicating its level (Error=red, Warning=yellow,
//! Success=green, Info=cyan). They auto-dismiss based on tick timers managed
//! by `AppState::tick_toasts()`.

use std::collections::VecDeque;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::tui::state::{Toast, ToastLevel};

/// Width of each toast widget (capped to terminal width).
const TOAST_WIDTH: u16 = 40;

/// Height of each toast widget (border + 1 line of text).
const TOAST_HEIGHT: u16 = 3;

/// Right margin from terminal edge.
const MARGIN_RIGHT: u16 = 1;

/// Bottom margin from terminal edge.
const MARGIN_BOTTOM: u16 = 2;

/// Map a `ToastLevel` to its display color.
pub fn level_color(level: ToastLevel) -> Color {
    match level {
        ToastLevel::Info => Color::Cyan,
        ToastLevel::Success => Color::Green,
        ToastLevel::Warning => Color::Yellow,
        ToastLevel::Error => Color::Red,
    }
}

/// Map a `ToastLevel` to its prefix icon.
fn level_icon(level: ToastLevel) -> &'static str {
    match level {
        ToastLevel::Info => "ℹ ",
        ToastLevel::Success => "✓ ",
        ToastLevel::Warning => "⚠ ",
        ToastLevel::Error => "✗ ",
    }
}

/// Render all active toasts as a bottom-right stack.
///
/// Toasts are rendered from bottom to top (newest at bottom). Each toast
/// is a small bordered box with a colored left border.
pub fn render(frame: &mut Frame, area: Rect, toasts: &VecDeque<Toast>) {
    if toasts.is_empty() {
        return;
    }

    let toast_width = TOAST_WIDTH.min(area.width.saturating_sub(MARGIN_RIGHT + 1));
    if toast_width < 10 || area.height < TOAST_HEIGHT + MARGIN_BOTTOM {
        return; // Terminal too small for toasts.
    }

    let x = area.x + area.width.saturating_sub(toast_width + MARGIN_RIGHT);

    // Render from bottom upward. Iterate in reverse so newest (back) is at bottom.
    for (i, toast) in toasts.iter().rev().enumerate() {
        let slot = i as u16;
        let y_offset = MARGIN_BOTTOM + slot * TOAST_HEIGHT;

        if y_offset + TOAST_HEIGHT > area.height {
            break; // No more room.
        }

        let y = area.y + area.height.saturating_sub(y_offset + TOAST_HEIGHT);
        let toast_area = Rect::new(x, y, toast_width, TOAST_HEIGHT);

        render_single_toast(frame, toast_area, toast);
    }
}

/// Render a single toast widget.
fn render_single_toast(frame: &mut Frame, area: Rect, toast: &Toast) {
    let color = level_color(toast.level);
    let icon = level_icon(toast.level);

    // Clear background behind this toast.
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Truncate message to fit (char-boundary-aware for multi-byte UTF-8).
    let max_msg_chars = inner.width as usize - icon.len();
    let msg = if toast.message.chars().count() > max_msg_chars {
        let truncated: String = toast
            .message
            .chars()
            .take(max_msg_chars.saturating_sub(1))
            .collect();
        format!("{truncated}…")
    } else {
        toast.message.clone()
    };

    let line = Line::from(vec![
        Span::styled(icon, Style::default().fg(color)),
        Span::styled(msg, Style::default().fg(Color::White)),
    ]);

    frame.render_widget(Paragraph::new(line), inner);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // T-TOAST-RENDER-01: level_color maps each level to the correct color.
    #[test]
    fn test_level_color_mapping() {
        assert_eq!(level_color(ToastLevel::Info), Color::Cyan);
        assert_eq!(level_color(ToastLevel::Success), Color::Green);
        assert_eq!(level_color(ToastLevel::Warning), Color::Yellow);
        assert_eq!(level_color(ToastLevel::Error), Color::Red);
    }

    // T-TOAST-RENDER-02: level_icon maps each level to a non-empty icon.
    #[test]
    fn test_level_icon_mapping() {
        assert!(!level_icon(ToastLevel::Info).is_empty());
        assert!(!level_icon(ToastLevel::Success).is_empty());
        assert!(!level_icon(ToastLevel::Warning).is_empty());
        assert!(!level_icon(ToastLevel::Error).is_empty());
    }

    // T-TOAST-RENDER-03: render with empty toasts is a no-op.
    #[test]
    fn test_render_empty_toasts() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let toasts: VecDeque<Toast> = VecDeque::new();

        terminal
            .draw(|frame| {
                render(frame, frame.area(), &toasts);
            })
            .unwrap();
        // Should not panic — no assertions needed beyond not crashing.
    }

    // T-TOAST-RENDER-04: render with toasts does not panic on small terminal.
    #[test]
    fn test_render_small_terminal_no_panic() {
        let backend = ratatui::backend::TestBackend::new(10, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut toasts = VecDeque::new();
        toasts.push_back(Toast {
            message: "hello".into(),
            level: ToastLevel::Error,
            ticks_remaining: 10,
            id: 1,
        });

        terminal
            .draw(|frame| {
                render(frame, frame.area(), &toasts);
            })
            .unwrap();
    }

    // T-TOAST-RENDER-05: render with multiple toasts does not panic.
    #[test]
    fn test_render_multiple_toasts() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut toasts = VecDeque::new();
        for i in 0..5 {
            toasts.push_back(Toast {
                message: format!("toast {i}"),
                level: match i % 4 {
                    0 => ToastLevel::Info,
                    1 => ToastLevel::Success,
                    2 => ToastLevel::Warning,
                    _ => ToastLevel::Error,
                },
                ticks_remaining: 10,
                id: i + 1,
            });
        }

        terminal
            .draw(|frame| {
                render(frame, frame.area(), &toasts);
            })
            .unwrap();
    }

    // T-TOAST-RENDER-06: Constant values are sensible.
    #[test]
    fn test_toast_dimensions() {
        assert!(TOAST_WIDTH >= 20);
        assert!(TOAST_HEIGHT >= 3);
        assert!(MARGIN_RIGHT >= 1);
        assert!(MARGIN_BOTTOM >= 1);
    }
}
