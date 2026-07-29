//! Modal keyboard interaction for the `AskUser` tool.

use std::collections::BTreeMap;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tui::theme::Theme;

const OTHER_LABEL: &str = "Other";

#[derive(Debug, Clone, Deserialize)]
struct AskUserQuestion {
    question: String,
    options: Vec<String>,
    #[serde(default)]
    multi_select: bool,
}

/// Result of confirming the current question.
#[derive(Debug, Clone, PartialEq)]
pub enum AskUserSubmit {
    Pending,
    Complete(Value),
}

/// Presentation state for one pending `AskUser` interaction.
#[derive(Debug, Clone, Default)]
pub struct AskUserState {
    interaction_id: Option<String>,
    questions: Vec<AskUserQuestion>,
    current: usize,
    focused: usize,
    selections: Vec<Vec<String>>,
    other_text: Vec<String>,
    editing_other: bool,
}

impl AskUserState {
    pub fn new(interaction_id: String, questions: Value) -> Result<Self, String> {
        let questions: Vec<AskUserQuestion> = serde_json::from_value(questions)
            .map_err(|error| format!("Invalid AskUser questions: {error}"))?;
        if questions.is_empty() {
            return Err("Invalid AskUser questions: the list is empty".into());
        }
        if questions
            .iter()
            .any(|question| question.question.is_empty() || question.options.is_empty())
        {
            return Err("Invalid AskUser questions: missing question text or options".into());
        }
        let count = questions.len();
        Ok(Self {
            interaction_id: Some(interaction_id),
            questions,
            current: 0,
            focused: 0,
            selections: vec![Vec::new(); count],
            other_text: vec![String::new(); count],
            editing_other: false,
        })
    }

    pub fn interaction_id(&self) -> Option<&str> {
        self.interaction_id.as_deref()
    }

    pub fn is_editing_other(&self) -> bool {
        self.editing_other
    }

    pub fn select_prev(&mut self) {
        if !self.editing_other {
            self.focused = self.focused.saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if !self.editing_other {
            let last = self
                .current_question()
                .map_or(0, |question| question.options.len());
            self.focused = self.focused.saturating_add(1).min(last);
        }
    }

    pub fn toggle_focused(&mut self) {
        let Some(question) = self.current_question() else {
            return;
        };
        if self.focused == question.options.len() {
            self.editing_other = true;
            return;
        }
        let option = question.options[self.focused].clone();
        let multi_select = question.multi_select;
        let selections = &mut self.selections[self.current];
        if multi_select {
            if let Some(position) = selections.iter().position(|selected| selected == &option) {
                selections.remove(position);
            } else {
                selections.push(option);
            }
        } else {
            selections.clear();
            selections.push(option);
            self.other_text[self.current].clear();
        }
    }

    pub fn push_other_char(&mut self, character: char) {
        if self.editing_other && character != '\n' && character != '\r' {
            self.other_text[self.current].push(character);
        }
    }

    pub fn pop_other_char(&mut self) {
        if self.editing_other {
            self.other_text[self.current].pop();
        }
    }

    pub fn submit(&mut self) -> AskUserSubmit {
        let Some(question) = self.current_question() else {
            return AskUserSubmit::Pending;
        };
        let multi_select = question.multi_select;
        let option_count = question.options.len();
        let focused_option = question.options.get(self.focused).cloned();
        if self.editing_other {
            if self.other_text[self.current].trim().is_empty() {
                return AskUserSubmit::Pending;
            }
            self.editing_other = false;
            if !multi_select {
                self.selections[self.current].clear();
            }
        } else if self.focused == option_count {
            self.editing_other = true;
            return AskUserSubmit::Pending;
        } else if !multi_select {
            let Some(option) = focused_option else {
                return AskUserSubmit::Pending;
            };
            self.selections[self.current] = vec![option];
            self.other_text[self.current].clear();
        } else if self.selections[self.current].is_empty()
            && self.other_text[self.current].is_empty()
        {
            self.toggle_focused();
            return AskUserSubmit::Pending;
        }

        if self.current + 1 < self.questions.len() {
            self.current += 1;
            self.focused = 0;
            AskUserSubmit::Pending
        } else {
            AskUserSubmit::Complete(self.answer_payload())
        }
    }

    fn current_question(&self) -> Option<&AskUserQuestion> {
        self.questions.get(self.current)
    }

    fn answer_payload(&self) -> Value {
        let answers: BTreeMap<&str, String> = self
            .questions
            .iter()
            .enumerate()
            .map(|(index, question)| {
                let mut answers = self.selections[index].clone();
                let other = self.other_text[index].trim();
                if !other.is_empty() {
                    answers.push(other.to_string());
                }
                (question.question.as_str(), answers.join(", "))
            })
            .collect();
        json!({ "answers": answers })
    }
}

pub fn render(frame: &mut Frame, area: Rect, state: &AskUserState, theme: &Theme) {
    let Some(question) = state.current_question() else {
        return;
    };
    let width = area.width.saturating_sub(4).clamp(24, 78);
    let height = (question.options.len() as u16 + 8)
        .min(area.height.saturating_sub(2))
        .max(10);
    let popup = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.input_border_focused()))
        .title(format!(
            " AskUser  {}/{} ",
            state.current + 1,
            state.questions.len()
        ))
        .title_style(
            Style::default()
                .fg(theme.input_title())
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(question.question.as_str())
            .style(Style::default().fg(theme.text()))
            .wrap(Wrap { trim: true }),
        rows[0],
    );

    let mut options = question.options.clone();
    options.push(OTHER_LABEL.into());
    let items = options.into_iter().enumerate().map(|(index, option)| {
        let focused = index == state.focused;
        let selected = if option == OTHER_LABEL {
            !state.other_text[state.current].is_empty()
        } else {
            state.selections[state.current].contains(&option)
        };
        let marker = if question.multi_select {
            if selected {
                "[x]"
            } else {
                "[ ]"
            }
        } else if selected {
            "(*)"
        } else {
            "( )"
        };
        let label = if option == OTHER_LABEL && state.editing_other {
            format!("{marker} Other: {}_", state.other_text[state.current])
        } else if option == OTHER_LABEL && !state.other_text[state.current].is_empty() {
            format!("{marker} Other: {}", state.other_text[state.current])
        } else {
            format!("{marker} {option}")
        };
        let style = if focused {
            Style::default()
                .fg(theme.panel_bg())
                .bg(theme.input_border_focused())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text())
        };
        ListItem::new(Line::from(Span::styled(label, style)))
    });
    frame.render_widget(List::new(items), rows[1]);
    let hint = if state.editing_other {
        " Type an answer  Enter confirm  Esc dismiss"
    } else if question.multi_select {
        " Up/Down navigate  Space toggle  Enter next  Esc dismiss"
    } else {
        " Up/Down navigate  Enter select  Esc dismiss"
    };
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(theme.muted())),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_select_advances_and_builds_answers() {
        let mut state = AskUserState::new(
            "interaction".into(),
            json!([
                {"question":"Language?","options":["Rust","Go"]},
                {"question":"Mode?","options":["Fast","Safe"]}
            ]),
        )
        .unwrap();
        assert_eq!(state.submit(), AskUserSubmit::Pending);
        state.select_next();
        assert_eq!(
            state.submit(),
            AskUserSubmit::Complete(json!({"answers":{"Language?":"Rust","Mode?":"Safe"}}))
        );
    }

    #[test]
    fn multi_select_and_other_are_combined() {
        let mut state = AskUserState::new(
            "interaction".into(),
            json!([{"question":"Features?","options":["Tests","Docs"],"multi_select":true}]),
        )
        .unwrap();
        state.toggle_focused();
        state.select_next();
        state.select_next();
        state.toggle_focused();
        for character in "Benchmarks".chars() {
            state.push_other_char(character);
        }
        assert_eq!(
            state.submit(),
            AskUserSubmit::Complete(json!({"answers":{"Features?":"Tests, Benchmarks"}}))
        );
    }

    #[test]
    fn rejects_invalid_question_payload() {
        assert!(AskUserState::new("interaction".into(), json!({})).is_err());
    }

    #[test]
    fn other_text_accepts_spaces() {
        let mut state = AskUserState::new(
            "interaction".into(),
            json!([{"question":"Choice?","options":["A","B"]}]),
        )
        .unwrap();
        state.select_next();
        state.select_next();
        assert_eq!(state.submit(), AskUserSubmit::Pending);
        for character in "custom choice".chars() {
            state.push_other_char(character);
        }
        assert_eq!(
            state.submit(),
            AskUserSubmit::Complete(json!({"answers":{"Choice?":"custom choice"}}))
        );
    }
}
