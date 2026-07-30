//! Compact active-run TODO queue rendered directly above the composer.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::theme::Theme;

const MAX_VISIBLE: usize = 4;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    todos: &[y_service::FollowUpMessage],
    send_shortcut: Option<&str>,
    theme: &Theme,
) {
    if area.height == 0 || todos.is_empty() {
        return;
    }
    let lines = styled_todo_lines(todos, send_shortcut, area.width as usize, theme);
    frame.render_widget(Paragraph::new(lines), area);
}

fn styled_todo_lines(
    todos: &[y_service::FollowUpMessage],
    send_shortcut: Option<&str>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(MAX_VISIBLE + 2);
    let shortcut = send_shortcut.map_or_else(
        || "  /queue manage".to_string(),
        |key| format!("  {key} send next  /queue manage"),
    );
    lines.push(Line::from(vec![
        Span::styled(
            format!(" TODO ({})", todos.len()),
            Style::default()
                .fg(theme.warning())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(shortcut, Style::default().fg(theme.muted())),
    ]));

    for (index, todo) in todos.iter().take(MAX_VISIBLE).enumerate() {
        let steering = todo.status == y_service::FollowUpStatus::Steering;
        let status = if steering { "steering" } else { "pending" };
        let marker = if steering { "*" } else { "-" };
        let prefix = format!(" {marker} {:>2}. [{status:<8}] ", index + 1);
        let available = width.saturating_sub(UnicodeWidthStr::width(prefix.as_str()));
        let normalized = todo.text.replace(['\n', '\r'], " ");
        let text = truncate(&normalized, available);
        let row_style = if steering {
            Style::default()
                .fg(theme.input_border_focused())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text())
        };
        lines.push(Line::from(vec![
            Span::styled(prefix, row_style),
            Span::styled(text, row_style),
        ]));
    }

    if todos.len() > MAX_VISIBLE {
        lines.push(Line::from(Span::styled(
            format!("   +{} more", todos.len() - MAX_VISIBLE),
            Style::default().fg(theme.muted()),
        )));
    }
    lines
}

#[cfg(test)]
fn todo_lines(
    todos: &[y_service::FollowUpMessage],
    send_shortcut: Option<&str>,
    width: usize,
) -> Vec<Line<'static>> {
    styled_todo_lines(todos, send_shortcut, width, &Theme::default())
}

fn truncate(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let content_width = width - 3;
    let mut output = String::new();
    let mut used = 0;
    for character in text.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used + character_width > content_width {
            break;
        }
        output.push(character);
        used += character_width;
    }
    output.push_str("...");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(id: &str, text: &str, status: y_service::FollowUpStatus) -> y_service::FollowUpMessage {
        y_service::FollowUpMessage {
            id: id.into(),
            text: text.into(),
            created_at: 0,
            status,
        }
    }

    #[test]
    fn test_todo_lines_show_status_shortcut_and_overflow() {
        let todos = vec![
            todo("1", "first", y_service::FollowUpStatus::Pending),
            todo("2", "second", y_service::FollowUpStatus::Steering),
            todo("3", "third", y_service::FollowUpStatus::Pending),
            todo("4", "fourth", y_service::FollowUpStatus::Pending),
            todo("5", "fifth", y_service::FollowUpStatus::Pending),
        ];

        let lines = todo_lines(&todos, Some("Ctrl+S"), 80);
        let text = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
            .collect::<String>();

        assert!(text.contains("TODO (5)"));
        assert!(text.contains("Ctrl+S send next"));
        assert!(text.contains("steering"));
        assert!(text.contains("+1 more"));
    }
}
