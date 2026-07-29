//! Scrollable keyboard help generated from the active semantic keymap.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::keys::{KeyContext, Keymap};

/// Render the help overlay.
pub fn render(frame: &mut Frame, area: Rect, keymap: &Keymap, scroll: u16) {
    let lines = help_lines(keymap);
    let popup_width = area.width.clamp(30, 72);
    let popup_height = area.height.clamp(10, lines.len() as u16 + 2);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);
    let visible_height = popup_height.saturating_sub(2);
    let max_scroll = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_sub(visible_height);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .title(" Help: active keymap ")
        .title_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll.min(max_scroll), 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, popup_area);
}

fn help_lines(keymap: &Keymap) -> Vec<Line<'static>> {
    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::White);
    let dim_style = Style::default().fg(Color::DarkGray);
    let sections = [
        ("Global", KeyContext::Global),
        ("Running", KeyContext::Streaming),
        ("Input: empty", KeyContext::NormalInputEmpty),
        ("Input: draft", KeyContext::NormalInputDraft),
        ("Conversation", KeyContext::NormalChat),
        ("Shell", KeyContext::Shell),
        ("Command palette", KeyContext::Command),
        ("Prompt backtrack", KeyContext::Select),
        ("Pickers", KeyContext::Picker),
        ("Follow-up queue", KeyContext::Queue),
        ("Tasks", KeyContext::Tasks),
        ("Help", KeyContext::Help),
    ];

    let mut lines = Vec::new();
    for (title, context) in sections {
        let entries = keymap.help_entries(context);
        if entries.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(format!("  {title}"), header_style)));
        for entry in entries {
            let keys = entry.keys.join(" / ");
            lines.push(keybinding_line(
                &keys,
                entry.description,
                key_style,
                desc_style,
            ));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Up/Down/PageUp/PageDown scroll  Esc closes",
        dim_style,
    )));
    lines
}

fn keybinding_line(
    key: &str,
    desc: &'static str,
    key_style: Style,
    desc_style: Style,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {key:<18}"), key_style),
        Span::styled(desc, desc_style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help_lines_are_generated_from_default_keymap() {
        let lines = help_lines(&Keymap::default());
        let text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
            .collect();

        assert!(text.contains("F1"));
        assert!(text.contains("Ctrl+Q"));
        assert!(text.contains("Cancel the active response"));
        assert!(text.contains("Follow-up queue"));
        assert!(!text.contains("Ctrl+H"));
    }

    #[test]
    fn test_help_uses_configured_override() {
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("show_help".to_string(), vec!["ctrl+?".to_string()]);
        let keymap = Keymap::with_overrides(overrides).unwrap();
        let text: String = help_lines(&keymap)
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
            .collect();

        assert!(text.contains("Ctrl+?"));
        assert!(!text.contains("F1"));
    }
}
