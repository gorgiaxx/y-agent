//! Modal prompts for guardrail permission escalations and plan reviews.
//!
//! Dangerous tool calls escalate to an `Ask` gate in y-service, which blocks
//! the turn until the presentation layer answers through
//! `session_state.pending_permissions`. Plan drafting can likewise block on a
//! manual review via `pending_plan_reviews`. Without a responder the turn
//! stalls until the HITL timeout (or forever for plan reviews), which
//! previously surfaced as "the run stops after the first assistant message".

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use y_service::chat_types::PlanReviewDecision;
use y_service::PermissionPromptResponse;

use crate::tui::theme::Theme;

/// Presentation state for one pending tool-permission prompt.
#[derive(Debug, Clone, Default)]
pub struct PermissionPromptState {
    request_id: Option<String>,
    tool_name: String,
    action_description: String,
    reason: String,
    content_preview: Option<String>,
    focused: usize,
}

impl PermissionPromptState {
    pub fn new(
        request_id: String,
        tool_name: String,
        action_description: String,
        reason: String,
        content_preview: Option<String>,
    ) -> Self {
        Self {
            request_id: Some(request_id),
            tool_name,
            action_description,
            reason,
            content_preview,
            focused: 0,
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    /// Options offered for this prompt. `ApproveAlways` persists an
    /// `exec_policy` rule and is only effective for `ShellExec`, matching the
    /// service-side contract.
    fn options(&self) -> Vec<(&'static str, PermissionPromptResponse)> {
        let mut options = vec![
            ("Allow once", PermissionPromptResponse::Approve),
            (
                "Allow all this session",
                PermissionPromptResponse::AllowAllForSession,
            ),
        ];
        if self.tool_name == "ShellExec" {
            options.push((
                "Always allow (save rule)",
                PermissionPromptResponse::ApproveAlways,
            ));
        }
        options.push(("Deny", PermissionPromptResponse::Deny));
        options
    }

    pub fn select_prev(&mut self) {
        self.focused = self.focused.saturating_sub(1);
    }

    pub fn select_next(&mut self) {
        self.focused = self
            .focused
            .saturating_add(1)
            .min(self.options().len().saturating_sub(1));
    }

    pub fn submit(&self) -> PermissionPromptResponse {
        self.options()
            .get(self.focused)
            .map_or(PermissionPromptResponse::Approve, |option| option.1)
    }

    /// Dismissal must still answer: the service blocks on a response, and a
    /// non-answer is indistinguishable from a timeout (also treated as Deny).
    pub fn dismiss() -> PermissionPromptResponse {
        PermissionPromptResponse::Deny
    }
}

/// Presentation state for one pending plan-review prompt.
#[derive(Debug, Clone, Default)]
pub struct PlanReviewPromptState {
    review_id: Option<String>,
    plan_title: String,
    plan_file: String,
    estimated_effort: String,
    overview: String,
    scope_in: Vec<String>,
    scope_out: Vec<String>,
    focused: usize,
}

impl PlanReviewPromptState {
    pub fn new(
        review_id: String,
        plan_title: String,
        plan_file: String,
        estimated_effort: String,
        overview: String,
        scope_in: Vec<String>,
        scope_out: Vec<String>,
    ) -> Self {
        Self {
            review_id: Some(review_id),
            plan_title,
            plan_file,
            estimated_effort,
            overview,
            scope_in,
            scope_out,
            focused: 0,
        }
    }

    pub fn review_id(&self) -> Option<&str> {
        self.review_id.as_deref()
    }

    pub fn select_prev(&mut self) {
        self.focused = self.focused.saturating_sub(1);
    }

    pub fn select_next(&mut self) {
        self.focused = self.focused.saturating_add(1).min(1);
    }

    pub fn submit(&self) -> PlanReviewDecision {
        if self.focused == 0 {
            PlanReviewDecision::Approve
        } else {
            PlanReviewDecision::Reject {
                feedback: String::new(),
            }
        }
    }

    /// The orchestrator waits on a decision with no timeout, so dismissal
    /// answers Reject instead of leaving the run parked forever.
    pub fn dismiss() -> PlanReviewDecision {
        PlanReviewDecision::Reject {
            feedback: String::new(),
        }
    }
}

/// Frame a centered popup sized to `content_height` rows of body content.
fn popup_rect(area: Rect, content_height: u16) -> Rect {
    let width = area.width.saturating_sub(4).clamp(24, 78);
    let height = (content_height + 2)
        .min(area.height.saturating_sub(2))
        .max(10);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn popup_block<'a>(title: String, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.input_border_focused()))
        .title(title)
        .title_style(
            Style::default()
                .fg(theme.input_title())
                .add_modifier(Modifier::BOLD),
        )
}

fn option_items<'a>(
    options: &[(&'static str, &'a str)],
    focused: usize,
    theme: &Theme,
) -> Vec<ListItem<'a>> {
    options
        .iter()
        .enumerate()
        .map(|(index, (label, detail))| {
            let focused_style = Style::default()
                .fg(theme.panel_bg())
                .bg(theme.input_border_focused())
                .add_modifier(Modifier::BOLD);
            let style = if index == focused {
                focused_style
            } else {
                Style::default().fg(theme.text())
            };
            let marker = if index == focused { "(*)" } else { "( )" };
            let text = if detail.is_empty() {
                format!("{marker} {label}")
            } else {
                format!("{marker} {label} — {detail}")
            };
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect()
}

pub fn render_permission(
    frame: &mut Frame,
    area: Rect,
    state: &PermissionPromptState,
    theme: &Theme,
) {
    if state.request_id.is_none() {
        return;
    }
    let options = state.options();
    let preview_lines: Vec<&str> = state
        .content_preview
        .as_deref()
        .map_or_else(Vec::new, |preview| preview.lines().take(6).collect());
    let preview_height = if preview_lines.is_empty() {
        0
    } else {
        (preview_lines.len() as u16 + 1).min(7)
    };
    // description (2) + preview + reason (1) + gap (1) + options + hint (2)
    let content_height = 2 + preview_height + 1 + 1 + options.len() as u16 + 2;
    let popup = popup_rect(area, content_height);
    frame.render_widget(Clear, popup);
    let block = popup_block(" Permission required ".to_string(), theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(preview_height),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(inner);

    let description = format!("{}: {}", state.tool_name, state.action_description);
    frame.render_widget(
        Paragraph::new(description)
            .style(Style::default().fg(theme.text()))
            .wrap(Wrap { trim: true }),
        rows[0],
    );

    if !preview_lines.is_empty() {
        let preview = preview_lines
            .iter()
            .map(|line| {
                Line::from(Span::styled(
                    (*line).to_string(),
                    Style::default().fg(theme.muted()),
                ))
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(preview), rows[1]);
    }

    if !state.reason.is_empty() {
        frame.render_widget(
            Paragraph::new(format!("Reason: {}", state.reason))
                .style(Style::default().fg(theme.muted())),
            rows[2],
        );
    }

    let option_details: Vec<(&'static str, &str)> =
        options.iter().map(|(label, _)| (*label, "")).collect();
    frame.render_widget(
        List::new(option_items(&option_details, state.focused, theme)),
        rows[4],
    );
    frame.render_widget(
        Paragraph::new(" Up/Down navigate  Enter select  Esc deny")
            .style(Style::default().fg(theme.muted())),
        rows[5],
    );
}

pub fn render_plan_review(
    frame: &mut Frame,
    area: Rect,
    state: &PlanReviewPromptState,
    theme: &Theme,
) {
    if state.review_id.is_none() {
        return;
    }
    let overview_lines: Vec<&str> = state.overview.lines().take(4).collect();
    let overview_height = (overview_lines.len() as u16 + 1).min(5);
    let scope_lines = (state.scope_in.len() + state.scope_out.len()).min(4) as u16;
    // title (2) + overview + scope + gap (1) + options (2) + hint (2)
    let content_height = 2 + overview_height + scope_lines + 1 + 2 + 2;
    let popup = popup_rect(area, content_height);
    frame.render_widget(Clear, popup);
    let block = popup_block(" Plan review ".to_string(), theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(overview_height),
            Constraint::Length(scope_lines),
            Constraint::Length(1),
            Constraint::Min(2),
            Constraint::Length(2),
        ])
        .split(inner);

    let title = if state.estimated_effort.is_empty() {
        format!("{}  ({})", state.plan_title, state.plan_file)
    } else {
        format!(
            "{}  ({}, effort: {})",
            state.plan_title, state.plan_file, state.estimated_effort
        )
    };
    frame.render_widget(
        Paragraph::new(title)
            .style(Style::default().fg(theme.text()))
            .wrap(Wrap { trim: true }),
        rows[0],
    );

    let overview = overview_lines
        .iter()
        .map(|line| {
            Line::from(Span::styled(
                (*line).to_string(),
                Style::default().fg(theme.muted()),
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(overview), rows[1]);

    let mut scope = Vec::new();
    for item in state.scope_in.iter().take(4) {
        scope.push(Line::from(Span::styled(
            format!("+ {item}"),
            Style::default().fg(theme.muted()),
        )));
    }
    for item in state
        .scope_out
        .iter()
        .take(4usize.saturating_sub(scope.len()))
    {
        scope.push(Line::from(Span::styled(
            format!("- {item}"),
            Style::default().fg(theme.muted()),
        )));
    }
    frame.render_widget(Paragraph::new(scope), rows[2]);

    let options = [("Approve plan", ""), ("Reject plan", "")];
    frame.render_widget(
        List::new(option_items(&options, state.focused, theme)),
        rows[4],
    );
    frame.render_widget(
        Paragraph::new(" Up/Down navigate  Enter select  Esc reject")
            .style(Style::default().fg(theme.muted())),
        rows[5],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permission(tool_name: &str) -> PermissionPromptState {
        PermissionPromptState::new(
            "req-1".into(),
            tool_name.into(),
            "run a shell command".into(),
            "dangerous tool".into(),
            Some("cargo test".into()),
        )
    }

    #[test]
    fn options_hide_always_for_non_shell_tools() {
        let state = permission("FileWrite");
        let responses: Vec<_> = state.options().into_iter().map(|option| option.1).collect();
        assert_eq!(
            responses,
            vec![
                PermissionPromptResponse::Approve,
                PermissionPromptResponse::AllowAllForSession,
                PermissionPromptResponse::Deny,
            ]
        );
    }

    #[test]
    fn options_include_always_for_shell_exec() {
        let state = permission("ShellExec");
        let responses: Vec<_> = state.options().into_iter().map(|option| option.1).collect();
        assert_eq!(
            responses,
            vec![
                PermissionPromptResponse::Approve,
                PermissionPromptResponse::AllowAllForSession,
                PermissionPromptResponse::ApproveAlways,
                PermissionPromptResponse::Deny,
            ]
        );
    }

    #[test]
    fn selection_clamps_at_last_option() {
        let mut state = permission("FileWrite");
        for _ in 0..10 {
            state.select_next();
        }
        assert_eq!(state.submit(), PermissionPromptResponse::Deny);
        state.select_prev();
        assert_eq!(state.submit(), PermissionPromptResponse::AllowAllForSession);
    }

    #[test]
    fn default_focus_approves() {
        let state = permission("FileEdit");
        assert_eq!(state.submit(), PermissionPromptResponse::Approve);
    }

    #[test]
    fn dismiss_denies() {
        assert_eq!(
            PermissionPromptState::dismiss(),
            PermissionPromptResponse::Deny
        );
    }

    fn plan_review() -> PlanReviewPromptState {
        PlanReviewPromptState::new(
            "rev-1".into(),
            "Refactor TUI".into(),
            ".claude/plans/x.md".into(),
            "medium".into(),
            "Split the renderer.".into(),
            vec!["crates/y-cli".into()],
            vec!["crates/y-service".into()],
        )
    }

    #[test]
    fn plan_review_submit_and_dismiss() {
        let mut state = plan_review();
        assert_eq!(state.submit(), PlanReviewDecision::Approve);
        state.select_next();
        state.select_next();
        assert_eq!(
            state.submit(),
            PlanReviewDecision::Reject {
                feedback: String::new()
            }
        );
        state.select_prev();
        assert_eq!(state.submit(), PlanReviewDecision::Approve);
        assert_eq!(
            PlanReviewPromptState::dismiss(),
            PlanReviewDecision::Reject {
                feedback: String::new()
            }
        );
    }

    #[test]
    fn render_permission_draws_tool_and_options() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = permission("ShellExec");
        let theme = Theme::default();
        terminal
            .draw(|frame| render_permission(frame, frame.area(), &state, &theme))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Permission required"));
        assert!(text.contains("ShellExec"));
        assert!(text.contains("cargo test"));
        assert!(text.contains("Always allow (save rule)"));
        assert!(text.contains("Deny"));
    }

    #[test]
    fn render_plan_review_draws_title_and_options() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = plan_review();
        let theme = Theme::default();
        terminal
            .draw(|frame| render_plan_review(frame, frame.area(), &state, &theme))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Plan review"));
        assert!(text.contains("Refactor TUI"));
        assert!(text.contains("Approve plan"));
        assert!(text.contains("Reject plan"));
    }

    /// Extract the full text of a `TestBackend` terminal buffer, one
    /// line per row, so assertions can search rendered content.
    fn buffer_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area();
        let mut text = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }
}
