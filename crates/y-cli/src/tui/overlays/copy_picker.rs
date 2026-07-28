use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::picker::{visible_range, PickerItem, PickerState};
use crate::tui::commands::copy::{CopyItem, CopyItemKind};
use crate::tui::theme::Theme;

/// A copy item plus its precomputed lowercase search haystacks, built once
/// when the picker is populated so per-keystroke filtering only runs
/// `contains` instead of lowercasing full transcripts on every key press.
#[derive(Debug, Clone)]
struct CopyPickerEntry {
    item: CopyItem,
    label_lower: String,
    detail_lower: String,
    content_lower: String,
}

impl CopyPickerEntry {
    fn new(item: CopyItem) -> Self {
        Self {
            label_lower: item.label.to_ascii_lowercase(),
            detail_lower: item.detail.to_ascii_lowercase(),
            content_lower: item.content.to_ascii_lowercase(),
            item,
        }
    }
}

impl PickerItem for CopyPickerEntry {
    fn matches(&self, query_lower: &str) -> bool {
        self.label_lower.contains(query_lower)
            || self.detail_lower.contains(query_lower)
            || self.content_lower.contains(query_lower)
    }
}

#[derive(Debug, Clone, Default)]
pub struct CopyPickerState {
    core: PickerState<CopyPickerEntry>,
}

impl CopyPickerState {
    pub fn new(items: Vec<CopyItem>) -> Self {
        Self {
            core: PickerState::new(items.into_iter().map(CopyPickerEntry::new).collect()),
        }
    }

    pub fn filtered_len(&self) -> usize {
        self.core.filtered_len()
    }

    pub fn selected_item(&self) -> Option<&CopyItem> {
        self.core.selected_item().map(|entry| &entry.item)
    }

    pub fn select_prev(&mut self) {
        self.core.select_prev();
    }

    pub fn select_next(&mut self) {
        self.core.select_next();
    }

    pub fn push_char(&mut self, character: char) {
        self.core.push_char(character);
    }

    pub fn pop_char(&mut self) {
        self.core.pop_char();
    }
}

pub fn render(frame: &mut Frame, area: Rect, picker: &CopyPickerState, theme: &Theme) {
    let area = picker_area(area);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.input_border_focused()))
        .title(" Copy content ")
        .title_style(
            Style::default()
                .fg(theme.input_title())
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Percentage(38),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(inner);

    let search = Line::from(vec![
        Span::styled(" Search: ", Style::default().fg(theme.muted())),
        Span::styled(picker.core.query(), Style::default().fg(theme.text())),
        Span::styled("_", Style::default().fg(theme.input_border_focused())),
    ]);
    frame.render_widget(Paragraph::new(search), rows[0]);

    let visible = visible_range(
        picker.filtered_len(),
        picker.core.selected(),
        rows[1].height as usize,
    );
    let list_items: Vec<ListItem> = visible
        .map(|position| {
            let item_index = picker.core.filtered()[position];
            let item = &picker.core.items()[item_index].item;
            let selected = position == picker.core.selected();
            let style = if selected {
                Style::default()
                    .fg(theme.panel_bg())
                    .bg(theme.input_border_focused())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text())
            };
            let detail_style = if selected {
                Style::default()
                    .fg(theme.panel_bg())
                    .bg(theme.input_border_focused())
            } else {
                Style::default().fg(theme.muted())
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {:<9} {}", kind_label(item.kind), item.label),
                    style,
                ),
                Span::styled(format!("  {}", item.detail), detail_style),
            ]))
        })
        .collect();
    if list_items.is_empty() {
        frame.render_widget(
            Paragraph::new(" No matching copy targets").style(Style::default().fg(theme.muted())),
            rows[1],
        );
    } else {
        frame.render_widget(List::new(list_items), rows[1]);
    }

    let preview = picker
        .selected_item()
        .map_or("Select an item to preview", |item| item.content.as_str());
    let preview_block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.muted()))
        .title(" Preview ");
    frame.render_widget(
        Paragraph::new(preview)
            .style(Style::default().fg(theme.text()))
            .block(preview_block)
            .wrap(Wrap { trim: false }),
        rows[2],
    );

    frame.render_widget(
        Paragraph::new(" Enter copy  Alt+Enter quote  Ctrl+L open path  Esc close")
            .style(Style::default().fg(theme.muted())),
        rows[3],
    );
}

fn picker_area(area: Rect) -> Rect {
    area
}

fn kind_label(kind: CopyItemKind) -> &'static str {
    match kind {
        CopyItemKind::AssistantResponse => "response",
        CopyItemKind::CodeBlock => "code",
        CopyItemKind::ToolInput => "tool in",
        CopyItemKind::ToolResult => "tool out",
        CopyItemKind::Command => "command",
        CopyItemKind::Path => "path",
        CopyItemKind::Transcript => "all",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, content: &str) -> CopyItem {
        CopyItem {
            kind: CopyItemKind::AssistantResponse,
            label: label.into(),
            detail: "detail".into(),
            content: content.into(),
        }
    }

    #[test]
    fn test_copy_picker_filters_labels_and_content() {
        let mut picker = CopyPickerState::new(vec![
            item("Response", "plain text"),
            item("ShellExec result", "cargo test passed"),
        ]);

        picker.push_char('c');
        picker.push_char('a');
        picker.push_char('r');
        picker.push_char('g');
        picker.push_char('o');

        assert_eq!(picker.filtered_len(), 1);
        assert_eq!(picker.selected_item().unwrap().label, "ShellExec result");
    }

    #[test]
    fn test_copy_picker_uses_entire_terminal_area() {
        let area = Rect::new(4, 3, 120, 40);
        assert_eq!(picker_area(area), area);
    }

    #[test]
    fn test_copy_picker_matches_case_insensitively() {
        let mut picker = CopyPickerState::new(vec![
            item("Response", "plain text"),
            item("ShellExec result", "cargo test passed"),
        ]);

        for character in "CARGO".chars() {
            picker.push_char(character);
        }

        assert_eq!(picker.filtered_len(), 1);
        assert_eq!(picker.selected_item().unwrap().label, "ShellExec result");
    }

    #[test]
    fn test_copy_picker_non_ascii_query_does_not_panic() {
        let mut picker = CopyPickerState::new(vec![
            item("Response", "plain text"),
            item("说明", "中文内容"),
        ]);

        for character in "中文".chars() {
            picker.push_char(character);
        }

        assert_eq!(picker.filtered_len(), 1);
        assert_eq!(picker.selected_item().unwrap().label, "说明");

        picker.pop_char();
        picker.pop_char();
        assert_eq!(picker.filtered_len(), 2);
    }
}
