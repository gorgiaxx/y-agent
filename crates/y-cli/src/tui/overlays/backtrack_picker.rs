//! Prompt backtrack selector for branching from an earlier user message.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::state::{AppState, MessageRole};
use crate::tui::theme::Theme;

/// Render the full-screen prompt backtrack selector.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused()))
        .title(" Backtrack to a prompt ")
        .title_style(
            Style::default()
                .fg(theme.title())
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(" Select a user prompt. Confirming creates a new branch; the original session is preserved.")
            .style(Style::default().fg(theme.muted()))
            .wrap(Wrap { trim: true }),
        rows[0],
    );

    let prompts: Vec<_> = state
        .messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == MessageRole::User)
        .collect();
    let selected_position = state
        .selected_message
        .and_then(|selected| prompts.iter().position(|(index, _)| *index == selected))
        .unwrap_or_else(|| prompts.len().saturating_sub(1));
    let visible = visible_range(prompts.len(), selected_position, rows[1].height as usize);
    let preview_width = usize::from(rows[1].width.saturating_sub(25)).max(8);
    let items: Vec<ListItem> = visible
        .map(|position| {
            let (message_index, message) = prompts[position];
            let selected = state.selected_message == Some(message_index);
            let style = if selected {
                Style::default()
                    .fg(theme.panel_bg())
                    .bg(theme.selected())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.normal())
            };
            ListItem::new(Line::from(Span::styled(
                format!(
                    " {:>3}  {}  {}",
                    position + 1,
                    message.timestamp.format("%Y-%m-%d %H:%M"),
                    prompt_preview(&message.content, preview_width)
                ),
                style,
            )))
        })
        .collect();
    frame.render_widget(List::new(items), rows[1]);

    let details = state.selected_user_message().map_or_else(
        || "No user prompt selected.".to_string(),
        |message| {
            format!(
                "Selected prompt\n{}\n\nEnter creates a branch before this prompt and restores it to the input editor.",
                prompt_preview(&message.content, 180)
            )
        },
    );
    frame.render_widget(
        Paragraph::new(details)
            .style(Style::default().fg(theme.text()))
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme.muted())),
            ),
        rows[2],
    );

    frame.render_widget(
        Paragraph::new(" Esc/Up older  Down newer  Enter branch & edit  q close")
            .style(Style::default().fg(theme.muted())),
        rows[3],
    );
}

fn visible_range(item_count: usize, selected: usize, height: usize) -> std::ops::Range<usize> {
    if item_count == 0 || height == 0 {
        return 0..0;
    }
    let selected = selected.min(item_count - 1);
    let start = selected.saturating_add(1).saturating_sub(height);
    let end = start.saturating_add(height).min(item_count);
    start..end
}

fn prompt_preview(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return "(empty prompt)".to_string();
    }

    let mut chars = normalized.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_none() {
        preview
    } else {
        let prefix = preview
            .chars()
            .take(max_chars.saturating_sub(3))
            .collect::<String>();
        format!("{prefix}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_preview_flattens_and_truncates_content() {
        assert_eq!(prompt_preview(" first\n  second ", 40), "first second");
        assert_eq!(prompt_preview("abcdefgh", 6), "abc...");
        assert_eq!(prompt_preview("  ", 10), "(empty prompt)");
    }

    #[test]
    fn visible_range_keeps_selection_on_screen() {
        assert_eq!(visible_range(10, 8, 4), 5..9);
        assert_eq!(visible_range(3, 2, 5), 0..3);
    }
}
