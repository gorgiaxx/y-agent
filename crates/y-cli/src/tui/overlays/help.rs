//! Help overlay: shows a categorized keyboard shortcut reference.
//!
//! Activated by `Ctrl+H` or the `/help` command. Provides a quick reference
//! for all available keybindings organized by mode.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

/// Render the help overlay.
pub fn render(frame: &mut Frame, area: Rect) {
    // Calculate popup size.
    let popup_width = area.width.min(60).max(30);
    let popup_height = area.height.min(30).max(10);

    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear background.
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .title(" Help — Keyboard Shortcuts ")
        .title_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );

    let lines = help_lines();
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, popup_area);
}

fn help_lines() -> Vec<Line<'static>> {
    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::White);
    let dim_style = Style::default().fg(Color::DarkGray);

    vec![
        Line::from(Span::styled("  Global", header_style)),
        keybinding_line("  Ctrl+Q/D/C", "Quit", key_style, desc_style),
        keybinding_line("  Ctrl+B    ", "Toggle sidebar", key_style, desc_style),
        keybinding_line("  Tab       ", "Cycle focus", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("  Normal Mode (Input)", header_style)),
        keybinding_line("  Enter     ", "Send message", key_style, desc_style),
        keybinding_line("  Shift+Enter", "New line", key_style, desc_style),
        keybinding_line("  :         ", "Command palette", key_style, desc_style),
        keybinding_line("  Esc       ", "Return to normal", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("  Normal Mode (Chat)", header_style)),
        keybinding_line("  j/k       ", "Scroll down/up", key_style, desc_style),
        keybinding_line("  PageUp/Down", "Page scroll", key_style, desc_style),
        keybinding_line("  i         ", "Return to input", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("  Command Mode", header_style)),
        keybinding_line("  Enter     ", "Execute command", key_style, desc_style),
        keybinding_line("  Esc       ", "Cancel", key_style, desc_style),
        keybinding_line("  ↑/↓       ", "Navigate commands", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("  Press Esc to close help", dim_style)),
    ]
}

fn keybinding_line<'a>(
    key: &'a str,
    desc: &'a str,
    key_style: Style,
    desc_style: Style,
) -> Line<'a> {
    Line::from(vec![
        Span::styled(key, key_style),
        Span::styled("  ", desc_style),
        Span::styled(desc, desc_style),
    ])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help_lines_not_empty() {
        let lines = help_lines();
        assert!(lines.len() >= 15, "help should have at least 15 lines");
    }

    #[test]
    fn test_help_lines_contain_keybindings() {
        let lines = help_lines();
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(text.contains("Ctrl+Q"));
        assert!(text.contains("Enter"));
        assert!(text.contains("Tab"));
    }
}
