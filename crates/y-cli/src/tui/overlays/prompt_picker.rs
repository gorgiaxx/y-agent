//! Full-screen selector for per-session prompt templates.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use y_service::UserPromptTemplate;

use super::picker::{preview, visible_range, PickerItem, PickerState};
use crate::tui::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptPickerSelection {
    Default,
    Template(UserPromptTemplate),
}

/// A picker choice plus its precomputed lowercase search fields, built once
/// at load time so per-keystroke filtering only runs `contains`.
#[derive(Debug, Clone)]
struct PromptPickerEntry {
    item: PromptPickerSelection,
    id_lower: String,
    name_lower: String,
    description_lower: String,
}

impl PromptPickerEntry {
    fn new(item: PromptPickerSelection) -> Self {
        let (id_lower, name_lower, description_lower) = match &item {
            PromptPickerSelection::Default => {
                ("default prompt".to_string(), String::new(), String::new())
            }
            PromptPickerSelection::Template(template) => (
                template.id.to_ascii_lowercase(),
                template.name.to_ascii_lowercase(),
                template
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            ),
        };
        Self {
            item,
            id_lower,
            name_lower,
            description_lower,
        }
    }
}

impl PickerItem for PromptPickerEntry {
    fn matches(&self, query_lower: &str) -> bool {
        self.id_lower.contains(query_lower)
            || self.name_lower.contains(query_lower)
            || self.description_lower.contains(query_lower)
    }
}

#[derive(Debug, Clone, Default)]
pub struct PromptPickerState {
    core: PickerState<PromptPickerEntry>,
}

impl PromptPickerState {
    pub fn new(templates: Vec<UserPromptTemplate>, active_template_id: Option<&str>) -> Self {
        let mut items = Vec::with_capacity(templates.len() + 1);
        items.push(PromptPickerEntry::new(PromptPickerSelection::Default));
        items.extend(
            templates
                .into_iter()
                .map(|template| PromptPickerEntry::new(PromptPickerSelection::Template(template))),
        );
        let selected = active_template_id
            .and_then(|active_id| {
                items.iter().position(|entry| {
                    matches!(&entry.item, PromptPickerSelection::Template(template) if template.id == active_id)
                })
            })
            .unwrap_or(0);
        let mut core = PickerState::new(items);
        core.set_selected(selected);
        Self { core }
    }

    pub fn filtered_len(&self) -> usize {
        self.core.filtered_len()
    }

    pub fn selected_choice(&self) -> Option<PromptPickerSelection> {
        self.core.selected_item().map(|entry| entry.item.clone())
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
            Span::styled(picker.core.query(), Style::default().fg(theme.text())),
            Span::styled("_", Style::default().fg(theme.border_focused())),
        ])),
        rows[0],
    );

    let visible = visible_range(
        picker.filtered_len(),
        picker.core.selected(),
        usize::from(rows[1].height),
    );
    let items: Vec<ListItem> = visible
        .map(|position| {
            let item = &picker.core.items()[picker.core.filtered()[position]].item;
            let selected = position == picker.core.selected();
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

fn item_label(item: &PromptPickerSelection) -> String {
    match item {
        PromptPickerSelection::Default => " Default prompt".to_string(),
        PromptPickerSelection::Template(template) => {
            format!(" {:<24} {}", preview(&template.name, 24), template.id)
        }
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

    #[test]
    fn test_prompt_picker_matches_case_insensitively() {
        let mut picker = PromptPickerState::new(
            vec![
                template("daily", "Daily Driver", "General coding"),
                template("review", "Reviewer", "Review changes"),
            ],
            None,
        );

        for character in "DAILY".chars() {
            picker.push_char(character);
        }

        assert_eq!(picker.filtered_len(), 1);
        assert!(matches!(
            picker.selected_choice(),
            Some(PromptPickerSelection::Template(ref selected)) if selected.id == "daily"
        ));
    }

    #[test]
    fn test_prompt_picker_non_ascii_query_does_not_panic() {
        let mut picker = PromptPickerState::new(
            vec![
                template("daily", "日常助手", "通用编码"),
                template("review", "Reviewer", "Review changes"),
            ],
            None,
        );

        for character in "日常".chars() {
            picker.push_char(character);
        }

        assert_eq!(picker.filtered_len(), 1);
        assert!(matches!(
            picker.selected_choice(),
            Some(PromptPickerSelection::Template(ref selected)) if selected.id == "daily"
        ));

        picker.pop_char();
        picker.pop_char();
        assert_eq!(picker.filtered_len(), 3);
    }
}
