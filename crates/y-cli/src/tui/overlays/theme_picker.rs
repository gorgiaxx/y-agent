//! Searchable `/theme` picker with live semantic-color preview.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::picker::{visible_range, PickerItem, PickerState};
use crate::tui::theme::{Theme, ThemeInfo};

#[derive(Debug, Clone)]
struct ThemePickerEntry {
    theme: ThemeInfo,
    search_lower: String,
}

impl ThemePickerEntry {
    fn new(theme: ThemeInfo) -> Self {
        let search_lower =
            format!("{} {} {}", theme.name, theme.label, theme.description).to_ascii_lowercase();
        Self {
            theme,
            search_lower,
        }
    }
}

impl PickerItem for ThemePickerEntry {
    fn matches(&self, query_lower: &str) -> bool {
        self.search_lower.contains(query_lower)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ThemePickerState {
    core: PickerState<ThemePickerEntry>,
}

impl ThemePickerState {
    pub fn new(themes: Vec<ThemeInfo>, active_name: &str) -> Self {
        let entries: Vec<ThemePickerEntry> =
            themes.into_iter().map(ThemePickerEntry::new).collect();
        let selected = entries
            .iter()
            .position(|entry| entry.theme.name == active_name)
            .unwrap_or(0);
        let mut core = PickerState::new(entries);
        core.set_selected(selected);
        Self { core }
    }

    pub fn filtered_len(&self) -> usize {
        self.core.filtered_len()
    }

    pub fn selected_name(&self) -> Option<&str> {
        self.core
            .selected_item()
            .map(|entry| entry.theme.name.as_str())
    }

    fn selected_theme(&self) -> Option<&ThemeInfo> {
        self.core.selected_item().map(|entry| &entry.theme)
    }

    pub fn select_prev(&mut self) {
        self.core.select_prev();
    }

    pub fn select_next(&mut self) {
        self.core.select_next();
    }

    pub fn page_prev(&mut self, page: usize) {
        self.core.page_prev(page);
    }

    pub fn page_next(&mut self, page: usize) {
        self.core.page_next(page);
    }

    pub fn push_char(&mut self, character: char) {
        self.core.push_char(character);
    }

    pub fn pop_char(&mut self) {
        self.core.pop_char();
    }
}

pub fn render(frame: &mut Frame, area: Rect, picker: &ThemePickerState, theme: &Theme) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused()))
        .style(Style::default().bg(theme.panel_bg()))
        .title(" Select color scheme ")
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
            Constraint::Length(7),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Search: ", Style::default().fg(theme.muted())),
            Span::styled(picker.core.query(), Style::default().fg(theme.text())),
            Span::styled("_", Style::default().fg(theme.border_focused())),
        ]))
        .style(Style::default().bg(theme.panel_bg())),
        rows[0],
    );

    let visible = visible_range(
        picker.filtered_len(),
        picker.core.selected(),
        usize::from(rows[1].height),
    );
    let items: Vec<ListItem> = visible
        .map(|position| {
            let info = &picker.core.items()[picker.core.filtered()[position]].theme;
            let selected = position == picker.core.selected();
            let marker = if info.is_custom { "custom" } else { "built-in" };
            let line = format!(" {:<24} {:<10} {}", info.label, marker, info.name);
            let style = if selected {
                Style::default()
                    .fg(theme.panel_bg())
                    .bg(theme.selected())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.normal()).bg(theme.panel_bg())
            };
            ListItem::new(Line::from(Span::styled(line, style)))
        })
        .collect();
    frame.render_widget(
        List::new(items).style(Style::default().bg(theme.panel_bg())),
        rows[1],
    );

    render_preview(frame, rows[2], picker, theme);
    frame.render_widget(
        Paragraph::new(" Up/Down preview  Type to search  Enter apply  Esc restore")
            .style(Style::default().fg(theme.muted()).bg(theme.panel_bg())),
        rows[3],
    );
}

fn render_preview(frame: &mut Frame, area: Rect, picker: &ThemePickerState, theme: &Theme) {
    let Some(info) = picker.selected_theme() else {
        frame.render_widget(
            Paragraph::new("No matching themes.")
                .style(Style::default().fg(theme.muted()).bg(theme.panel_bg())),
            area,
        );
        return;
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} ", info.label),
                Style::default()
                    .fg(theme.title())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                info.description.as_str(),
                Style::default().fg(theme.muted()),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            swatch(" Primary ", theme.assistant_accent(), theme.panel_bg()),
            Span::raw("  "),
            swatch(" Success ", theme.success(), theme.panel_bg()),
            Span::raw("  "),
            swatch(" Warning ", theme.warning(), theme.panel_bg()),
            Span::raw("  "),
            swatch(" Error ", theme.error(), theme.panel_bg()),
        ]),
        Line::from(vec![
            Span::styled(" You  ", Style::default().fg(theme.user_accent())),
            Span::styled(
                "Update the current implementation.",
                Style::default().fg(theme.text()),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Agent  ", Style::default().fg(theme.assistant_accent())),
            Span::styled(
                "Working through the changes now.",
                Style::default().fg(theme.text()),
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(theme.panel_bg()))
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme.muted()))
                    .title(" Live preview "),
            ),
        area,
    );
}

fn swatch(
    label: &'static str,
    background: ratatui::style::Color,
    foreground: ratatui::style::Color,
) -> Span<'static> {
    Span::styled(label, Style::default().fg(foreground).bg(background))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::ThemeInfo;

    fn themes() -> Vec<ThemeInfo> {
        vec![
            ThemeInfo {
                name: "default".into(),
                label: "Y Agent Default".into(),
                description: "Original palette".into(),
                is_custom: false,
            },
            ThemeInfo {
                name: "nord".into(),
                label: "Nord".into(),
                description: "Cool palette".into(),
                is_custom: false,
            },
            ThemeInfo {
                name: "ember".into(),
                label: "Custom: ember".into(),
                description: "User theme".into(),
                is_custom: true,
            },
        ]
    }

    #[test]
    fn picker_preselects_active_theme_and_navigates() {
        let mut picker = ThemePickerState::new(themes(), "nord");

        assert_eq!(picker.selected_name(), Some("nord"));
        picker.select_next();
        assert_eq!(picker.selected_name(), Some("ember"));
        picker.select_prev();
        assert_eq!(picker.selected_name(), Some("nord"));
    }

    #[test]
    fn picker_searches_names_labels_and_descriptions() {
        let mut picker = ThemePickerState::new(themes(), "default");

        for character in "user".chars() {
            picker.push_char(character);
        }

        assert_eq!(picker.filtered_len(), 1);
        assert_eq!(picker.selected_name(), Some("ember"));
    }
}
