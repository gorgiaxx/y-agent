use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::commands::copy::{CopyItem, CopyItemKind};
use crate::tui::theme::Theme;

#[derive(Debug, Clone, Default)]
pub struct CopyPickerState {
    items: Vec<CopyItem>,
    filtered: Vec<usize>,
    selected: usize,
    query: String,
}

impl CopyPickerState {
    pub fn new(items: Vec<CopyItem>) -> Self {
        let filtered = (0..items.len()).collect();
        Self {
            items,
            filtered,
            selected: 0,
            query: String::new(),
        }
    }

    pub fn filtered_len(&self) -> usize {
        self.filtered.len()
    }

    pub fn selected_item(&self) -> Option<&CopyItem> {
        self.filtered
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    pub fn push_char(&mut self, character: char) {
        self.query.push(character);
        self.update_filter();
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.update_filter();
    }

    fn update_filter(&mut self) {
        let query = self.query.to_ascii_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                query.is_empty()
                    || item.label.to_ascii_lowercase().contains(&query)
                    || item.detail.to_ascii_lowercase().contains(&query)
                    || item.content.to_ascii_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
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
        Span::styled(&picker.query, Style::default().fg(theme.text())),
        Span::styled("_", Style::default().fg(theme.input_border_focused())),
    ]);
    frame.render_widget(Paragraph::new(search), rows[0]);

    let visible = visible_range(
        picker.filtered.len(),
        picker.selected,
        rows[1].height as usize,
    );
    let list_items: Vec<ListItem> = visible
        .map(|position| {
            let item_index = picker.filtered[position];
            let item = &picker.items[item_index];
            let selected = position == picker.selected;
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
        Paragraph::new(" Up/Down navigate  Type to search  Enter copy  Esc close")
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
        CopyItemKind::Transcript => "all",
    }
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
}
