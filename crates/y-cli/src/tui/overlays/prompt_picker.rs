//! Full-screen selector for per-session prompt templates.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use y_service::UserPromptTemplate;

use crate::tui::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptPickerSelection {
    Default,
    Template(UserPromptTemplate),
}

#[derive(Debug, Clone, Default)]
pub struct PromptPickerState {
    items: Vec<PromptPickerSelection>,
    filtered: Vec<usize>,
    selected: usize,
    query: String,
}

impl PromptPickerState {
    pub fn new(templates: Vec<UserPromptTemplate>, active_template_id: Option<&str>) -> Self {
        let mut items = Vec::with_capacity(templates.len() + 1);
        items.push(PromptPickerSelection::Default);
        items.extend(templates.into_iter().map(PromptPickerSelection::Template));
        let filtered: Vec<usize> = (0..items.len()).collect();
        let selected = active_template_id
            .and_then(|active_id| {
                items.iter().position(|item| {
                    matches!(item, PromptPickerSelection::Template(template) if template.id == active_id)
                })
            })
            .unwrap_or(0);
        Self {
            items,
            filtered,
            selected,
            query: String::new(),
        }
    }

    pub fn filtered_len(&self) -> usize {
        self.filtered.len()
    }

    pub fn selected_choice(&self) -> Option<PromptPickerSelection> {
        self.filtered
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
            .cloned()
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
            .filter(|(_, item)| item_matches(item, &query))
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
    }
}

pub fn render(frame: &mut Frame, area: Rect, picker: &PromptPickerState, theme: &Theme) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused()))
        .title(" Session prompt template ")
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
            Span::styled(&picker.query, Style::default().fg(theme.text())),
            Span::styled("_", Style::default().fg(theme.border_focused())),
        ])),
        rows[0],
    );

    let visible = visible_range(
        picker.filtered.len(),
        picker.selected,
        usize::from(rows[1].height),
    );
    let items: Vec<ListItem> = visible
        .map(|position| {
            let item = &picker.items[picker.filtered[position]];
            let selected = position == picker.selected;
            let style = if selected {
                Style::default()
                    .fg(theme.panel_bg())
                    .bg(theme.selected())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.normal())
            };
            ListItem::new(Line::from(Span::styled(item_label(item), style)))
        })
        .collect();
    frame.render_widget(List::new(items), rows[1]);

    let details = picker.selected_choice().map_or_else(
        || "No matching prompt templates.".to_string(),
        |choice| match choice {
            PromptPickerSelection::Default => {
                "Use default prompt\nClear the per-session prompt override and use the built-in prompt composition."
                    .to_string()
            }
            PromptPickerSelection::Template(template) => format!(
                "{}\nID: {}\nSections: {}\n\n{}",
                template.description.as_deref().unwrap_or(&template.name),
                template.id,
                if template.prompt_section_ids.is_empty() {
                    "none".to_string()
                } else {
                    template.prompt_section_ids.join(", ")
                },
                preview(&template.system_prompt, 180)
            ),
        },
    );
    frame.render_widget(
        Paragraph::new(details)
            .style(Style::default().fg(theme.text()))
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme.muted()))
                    .title(" Prompt details "),
            ),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(" Up/Down navigate  Type to search  Enter apply  Esc close")
            .style(Style::default().fg(theme.muted())),
        rows[3],
    );
}

fn item_matches(item: &PromptPickerSelection, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    match item {
        PromptPickerSelection::Default => "default prompt".contains(query),
        PromptPickerSelection::Template(template) => {
            template.id.to_ascii_lowercase().contains(query)
                || template.name.to_ascii_lowercase().contains(query)
                || template
                    .description
                    .as_deref()
                    .is_some_and(|description| description.to_ascii_lowercase().contains(query))
        }
    }
}

fn item_label(item: &PromptPickerSelection) -> String {
    match item {
        PromptPickerSelection::Default => " Default prompt".to_string(),
        PromptPickerSelection::Template(template) => {
            format!(" {:<24} {}", preview(&template.name, 24), template.id)
        }
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

fn preview(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_none() {
        preview
    } else {
        format!(
            "{}...",
            preview
                .chars()
                .take(max_chars.saturating_sub(3))
                .collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(id: &str, name: &str, description: &str) -> y_service::UserPromptTemplate {
        y_service::UserPromptTemplate {
            id: id.into(),
            name: name.into(),
            description: Some(description.into()),
            system_prompt: format!("System prompt for {name}"),
            prompt_section_ids: vec!["core.datetime".into()],
        }
    }

    #[test]
    fn test_prompt_picker_selects_active_template_and_default() {
        let templates = vec![
            template("daily", "Daily Driver", "General coding"),
            template("review", "Reviewer", "Review changes"),
        ];
        let picker = PromptPickerState::new(templates.clone(), Some("review"));
        assert!(matches!(
            picker.selected_choice(),
            Some(PromptPickerSelection::Template(ref selected)) if selected.id == "review"
        ));

        let default_picker = PromptPickerState::new(templates, None);
        assert!(matches!(
            default_picker.selected_choice(),
            Some(PromptPickerSelection::Default)
        ));
    }

    #[test]
    fn test_prompt_picker_filters_names_ids_and_descriptions() {
        let mut picker = PromptPickerState::new(
            vec![
                template("daily", "Daily Driver", "General coding"),
                template("review", "Reviewer", "Review changes"),
            ],
            None,
        );

        for character in "changes".chars() {
            picker.push_char(character);
        }

        assert_eq!(picker.filtered_len(), 1);
        assert!(matches!(
            picker.selected_choice(),
            Some(PromptPickerSelection::Template(ref selected)) if selected.id == "review"
        ));
    }
}
