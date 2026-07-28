//! `/tasks` overlay: lists active subagents (delegated agent executions) and
//! runtime-managed background tasks, with kill and inline output preview.
//!
//! Built on the shared [`PickerState`] core like the other picker overlays,
//! but key-driven without a search row. Both sections live in one flat list
//! with non-selectable header rows; navigation skips the headers.

use std::time::Duration;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
use y_agent::agent::pool::ActiveDelegation;
use y_service::{BackgroundTaskInfo, BackgroundTaskSnapshot};

use super::picker::{preview, truncate, visible_range, PickerItem, PickerState};
use crate::tui::theme::Theme;

/// Maximum number of output lines kept in the inline preview pane.
pub const PREVIEW_MAX_LINES: usize = 10;

/// Height of the inline preview pane, including its top border.
const PREVIEW_PANE_HEIGHT: u16 = PREVIEW_MAX_LINES as u16 + 2;

/// One row in the `/tasks` overlay list.
#[derive(Debug, Clone)]
pub enum TasksRow {
    /// Non-selectable section header with a count, e.g. "Subagents (2)".
    Header(String),
    /// A delegated subagent execution currently in flight.
    Subagent(ActiveDelegation),
    /// A runtime-managed background task.
    Task(BackgroundTaskInfo),
}

/// Inline output preview for a background task row.
#[derive(Debug, Clone)]
pub struct TaskPreview {
    /// Process the snapshot belongs to.
    process_id: String,
    /// Rendered output lines (tail of stdout/stderr).
    lines: Vec<String>,
}

/// The outcome of a kill request for an overlay row, kept pure so the key
/// handling can be tested without services.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillEffect<'a> {
    /// Kill the background task with this process id.
    KillTask(&'a str),
    /// The row exists but cannot be killed from this overlay (subagent or
    /// section header).
    NotKillable,
    /// No row selected; ignore the keypress.
    Noop,
}

/// Route a kill request for `row`.
pub fn kill_effect(row: Option<&TasksRow>) -> KillEffect<'_> {
    match row {
        Some(TasksRow::Task(task)) => KillEffect::KillTask(task.process_id.as_str()),
        Some(TasksRow::Subagent(_) | TasksRow::Header(_)) => KillEffect::NotKillable,
        None => KillEffect::Noop,
    }
}

/// Format a duration compactly for overlay rows, e.g. "5s", "1m05s", "1h02m".
pub fn format_elapsed(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Compact status label for a task row; finished tasks carry their exit code
/// or error instead of the bare status word.
pub fn task_status_label(task: &BackgroundTaskInfo) -> String {
    match task.status.as_str() {
        "running" => format!(
            "running {}",
            format_elapsed(Duration::from_millis(task.duration_ms))
        ),
        "completed" => task
            .exit_code
            .map_or_else(|| "completed".to_string(), |code| format!("exit {code}")),
        "failed" => task.error.as_deref().map_or_else(
            || "failed".to_string(),
            |error| format!("failed: {}", preview(error, 16)),
        ),
        other => other.to_string(),
    }
}

/// Build the inline preview for a task snapshot: the tail of stdout followed
/// by the tail of stderr (behind a marker), capped at [`PREVIEW_MAX_LINES`].
pub fn preview_lines(snapshot: &BackgroundTaskSnapshot) -> Vec<String> {
    let mut lines: Vec<String> = snapshot.stdout.lines().map(String::from).collect();
    if !snapshot.stderr.trim().is_empty() {
        lines.push("-- stderr --".to_string());
        lines.extend(snapshot.stderr.lines().map(String::from));
    }
    let overflow = lines.len().saturating_sub(PREVIEW_MAX_LINES);
    let tail = lines.split_off(overflow);
    if tail.is_empty() {
        vec!["(no output)".to_string()]
    } else {
        tail
    }
}

/// A row plus its precomputed lowercase match text, mirroring the entry
/// wrappers of the other pickers (no search row here, but [`PickerState`]
/// requires the trait).
#[derive(Debug, Clone)]
struct TasksPickerEntry {
    row: TasksRow,
    text_lower: String,
}

impl TasksPickerEntry {
    fn new(row: TasksRow) -> Self {
        let text = match &row {
            TasksRow::Header(title) => title.clone(),
            TasksRow::Subagent(delegation) => delegation.agent_name.clone(),
            TasksRow::Task(task) => format!(
                "{} {} {} {}",
                task.process_id, task.backend, task.status, task.command
            ),
        };
        Self {
            row,
            text_lower: text.to_ascii_lowercase(),
        }
    }

    /// Whether the cursor may rest on this row (section headers may not).
    fn selectable(&self) -> bool {
        matches!(self.row, TasksRow::Subagent(_) | TasksRow::Task(_))
    }
}

impl PickerItem for TasksPickerEntry {
    fn matches(&self, query_lower: &str) -> bool {
        self.text_lower.contains(query_lower)
    }
}

/// Build the flat row list: subagent section first, then background tasks,
/// each behind a header carrying the entry count.
fn build_entries(
    subagents: Vec<ActiveDelegation>,
    tasks: Vec<BackgroundTaskInfo>,
) -> Vec<TasksPickerEntry> {
    let mut entries = Vec::with_capacity(subagents.len() + tasks.len() + 2);
    entries.push(TasksPickerEntry::new(TasksRow::Header(format!(
        "Subagents ({})",
        subagents.len()
    ))));
    entries.extend(
        subagents
            .into_iter()
            .map(|delegation| TasksPickerEntry::new(TasksRow::Subagent(delegation))),
    );
    entries.push(TasksPickerEntry::new(TasksRow::Header(format!(
        "Background tasks ({})",
        tasks.len()
    ))));
    entries.extend(
        tasks
            .into_iter()
            .map(|task| TasksPickerEntry::new(TasksRow::Task(task))),
    );
    entries
}

/// State for the `/tasks` overlay.
#[derive(Debug, Clone, Default)]
pub struct TasksPickerState {
    core: PickerState<TasksPickerEntry>,
    /// Inline output preview for the toggled task row, if any.
    preview: Option<TaskPreview>,
}

impl TasksPickerState {
    /// Populate the overlay: subagents first, then background tasks.
    pub fn new(subagents: Vec<ActiveDelegation>, tasks: Vec<BackgroundTaskInfo>) -> Self {
        let mut state = Self {
            core: PickerState::new(build_entries(subagents, tasks)),
            preview: None,
        };
        // Land the cursor on the first selectable row (row 0 is a header).
        if let Some(first) = state.first_selectable() {
            state.core.set_selected(first);
        }
        state
    }

    /// Replace the row data on a periodic refresh, keeping the cursor
    /// position (clamped to a selectable row) and dropping the preview when
    /// its task vanished from the list.
    pub fn replace_rows(
        &mut self,
        subagents: Vec<ActiveDelegation>,
        tasks: Vec<BackgroundTaskInfo>,
    ) {
        let selected = self.core.selected();
        let still_listed = tasks
            .iter()
            .any(|task| Some(task.process_id.as_str()) == self.preview_process_id());
        self.core = PickerState::new(build_entries(subagents, tasks));
        self.core.set_selected(selected);
        self.clamp_selection();
        if !still_listed {
            self.preview = None;
        }
    }

    /// Number of visible rows, including section headers.
    pub fn filtered_len(&self) -> usize {
        self.core.filtered_len()
    }

    /// The row under the cursor, if the list is non-empty.
    pub fn selected_row(&self) -> Option<&TasksRow> {
        self.core.selected_item().map(|entry| &entry.row)
    }

    /// The background task under the cursor, if a task row is selected.
    pub fn selected_task(&self) -> Option<&BackgroundTaskInfo> {
        match self.selected_row() {
            Some(TasksRow::Task(task)) => Some(task),
            _ => None,
        }
    }

    /// Move the selection up to the previous selectable row, skipping
    /// section headers.
    pub fn select_prev(&mut self) {
        let mut candidate = self.core.selected();
        while candidate > 0 {
            candidate -= 1;
            if self.selectable_at(candidate) {
                self.core.set_selected(candidate);
                return;
            }
        }
    }

    /// Move the selection down to the next selectable row, skipping section
    /// headers.
    pub fn select_next(&mut self) {
        let len = self.core.filtered_len();
        let mut candidate = self.core.selected();
        while candidate + 1 < len {
            candidate += 1;
            if self.selectable_at(candidate) {
                self.core.set_selected(candidate);
                return;
            }
        }
    }

    /// The active inline preview, if a task's output is toggled open.
    pub fn preview(&self) -> Option<&TaskPreview> {
        self.preview.as_ref()
    }

    /// Process id of the task whose output is previewed, if any.
    pub fn preview_process_id(&self) -> Option<&str> {
        self.preview
            .as_ref()
            .map(|preview| preview.process_id.as_str())
    }

    /// Open the inline preview for a task snapshot (replaces any open one).
    pub fn set_preview(&mut self, snapshot: &BackgroundTaskSnapshot) {
        self.preview = Some(TaskPreview {
            process_id: snapshot.process_id.clone(),
            lines: preview_lines(snapshot),
        });
    }

    /// Close the inline preview.
    pub fn clear_preview(&mut self) {
        self.preview = None;
    }

    fn selectable_at(&self, position: usize) -> bool {
        self.core
            .filtered()
            .get(position)
            .is_some_and(|index| self.core.items()[*index].selectable())
    }

    fn first_selectable(&self) -> Option<usize> {
        (0..self.core.filtered_len()).find(|&position| self.selectable_at(position))
    }

    /// Clamp the cursor into range and onto a selectable row, preferring the
    /// nearest selectable row at or above the requested position.
    fn clamp_selection(&mut self) {
        let len = self.core.filtered_len();
        if len == 0 {
            self.core.set_selected(0);
            return;
        }
        let position = self.core.selected().min(len - 1);
        self.core.set_selected(position);
        if self.selectable_at(position) {
            return;
        }
        if let Some(up) = (0..position).rev().find(|&pos| self.selectable_at(pos)) {
            self.core.set_selected(up);
        } else if let Some(down) = (position + 1..len).find(|&pos| self.selectable_at(pos)) {
            self.core.set_selected(down);
        }
    }
}

/// Row highlight for selectable rows; headers keep their own style.
fn row_style(selected: bool, theme: &Theme) -> Style {
    if selected {
        Style::default()
            .fg(theme.panel_bg())
            .bg(theme.input_border_focused())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text())
    }
}

pub fn render(frame: &mut Frame, area: Rect, picker: &TasksPickerState, theme: &Theme) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.input_border_focused()))
        .title(" Tasks ")
        .title_style(
            Style::default()
                .fg(theme.input_title())
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let has_preview = picker.preview().is_some();
    let constraints = if has_preview {
        vec![
            Constraint::Min(3),
            Constraint::Length(PREVIEW_PANE_HEIGHT),
            Constraint::Length(1),
        ]
    } else {
        vec![Constraint::Min(3), Constraint::Length(1)]
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let visible = visible_range(
        picker.filtered_len(),
        picker.core.selected(),
        rows[0].height as usize,
    );
    // Row layout: " pid      backend  status                  command".
    let command_width = (rows[0].width as usize).saturating_sub(44);
    let items: Vec<ListItem> = visible
        .map(|position| {
            let entry = &picker.core.items()[picker.core.filtered()[position]];
            let selected = position == picker.core.selected() && entry.selectable();
            match &entry.row {
                TasksRow::Header(title) => ListItem::new(Line::from(Span::styled(
                    format!(" {title}"),
                    Style::default()
                        .fg(theme.input_title())
                        .add_modifier(Modifier::BOLD),
                ))),
                TasksRow::Subagent(delegation) => ListItem::new(Line::from(Span::styled(
                    format!(
                        "  {:<24}  running {}",
                        truncate(&delegation.agent_name, 24),
                        format_elapsed(delegation.start_time.elapsed()),
                    ),
                    row_style(selected, theme),
                ))),
                TasksRow::Task(task) => ListItem::new(Line::from(Span::styled(
                    format!(
                        "  {:<8}  {:<8}  {:<22}  {}",
                        truncate(&task.process_id, 8),
                        truncate(&task.backend, 8),
                        truncate(&task_status_label(task), 22),
                        preview(&task.command, command_width),
                    ),
                    row_style(selected, theme),
                ))),
            }
        })
        .collect();
    frame.render_widget(List::new(items), rows[0]);

    if let Some(preview) = picker.preview() {
        let preview_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme.muted()))
            .title(format!(" output: {} ", preview.process_id))
            .title_style(Style::default().fg(theme.muted()));
        let lines: Vec<Line> = preview
            .lines
            .iter()
            .map(|line| Line::from(line.clone()))
            .collect();
        frame.render_widget(
            Paragraph::new(lines)
                .block(preview_block)
                .style(Style::default().fg(theme.text())),
            rows[1],
        );
    }

    frame.render_widget(
        Paragraph::new(" Up/Down navigate  Enter output  d kill  r refresh  Esc close")
            .style(Style::default().fg(theme.muted())),
        rows[rows.len() - 1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn delegation(id: &str, agent_name: &str) -> ActiveDelegation {
        ActiveDelegation {
            id: id.into(),
            agent_name: agent_name.into(),
            start_time: Instant::now(),
        }
    }

    fn task(process_id: &str, status: &str, command: &str) -> BackgroundTaskInfo {
        BackgroundTaskInfo {
            process_id: process_id.into(),
            backend: "native".into(),
            command: command.into(),
            working_dir: None,
            status: status.into(),
            exit_code: None,
            error: None,
            duration_ms: 0,
        }
    }

    fn snapshot(process_id: &str, stdout: &str, stderr: &str) -> BackgroundTaskSnapshot {
        BackgroundTaskSnapshot {
            process_id: process_id.into(),
            backend: "native".into(),
            status: "running".into(),
            exit_code: None,
            error: None,
            stdout: stdout.into(),
            stderr: stderr.into(),
            duration_ms: 0,
        }
    }

    #[test]
    fn test_tasks_picker_groups_sections_with_headers() {
        let picker = TasksPickerState::new(
            vec![delegation("d-1", "researcher")],
            vec![task("proc-1", "running", "sleep 60")],
        );

        // Two headers plus one row per entry.
        assert_eq!(picker.filtered_len(), 4);
        assert!(matches!(
            picker.core.items()[0].row,
            TasksRow::Header(ref title) if title == "Subagents (1)"
        ));
        assert!(matches!(
            picker.core.items()[2].row,
            TasksRow::Header(ref title) if title == "Background tasks (1)"
        ));
        // The cursor starts on the first selectable row (the subagent).
        assert!(matches!(
            picker.selected_row(),
            Some(TasksRow::Subagent(ref d)) if d.agent_name == "researcher"
        ));
    }

    #[test]
    fn test_tasks_picker_navigation_skips_headers_and_clamps() {
        let mut picker = TasksPickerState::new(
            vec![delegation("d-1", "researcher")],
            vec![
                task("proc-1", "running", "sleep 60"),
                task("proc-2", "running", "make build"),
            ],
        );

        // Rows: [Header, Subagent, Header, Task, Task]; start on the subagent.
        assert!(matches!(picker.selected_row(), Some(TasksRow::Subagent(_))));
        picker.select_next();
        assert!(
            matches!(picker.selected_row(), Some(TasksRow::Task(ref task)) if task.process_id == "proc-1"),
            "must skip the tasks header"
        );
        picker.select_next();
        assert!(
            matches!(picker.selected_row(), Some(TasksRow::Task(ref task)) if task.process_id == "proc-2")
        );
        picker.select_next();
        assert!(
            matches!(picker.selected_row(), Some(TasksRow::Task(ref task)) if task.process_id == "proc-2"),
            "clamps at the last selectable row"
        );

        picker.select_prev();
        assert!(
            matches!(picker.selected_row(), Some(TasksRow::Task(ref task)) if task.process_id == "proc-1")
        );
        picker.select_prev();
        assert!(
            matches!(picker.selected_row(), Some(TasksRow::Subagent(_))),
            "must skip the tasks header upwards"
        );
        picker.select_prev();
        assert!(
            matches!(picker.selected_row(), Some(TasksRow::Subagent(_))),
            "clamps at the first selectable row"
        );
    }

    #[test]
    fn test_tasks_picker_without_subagents_starts_on_first_task() {
        let picker = TasksPickerState::new(Vec::new(), vec![task("proc-1", "running", "sleep 60")]);

        // Rows: [Header, Header, Task].
        assert!(matches!(
            picker.selected_row(),
            Some(TasksRow::Task(ref task)) if task.process_id == "proc-1"
        ));
    }

    #[test]
    fn test_tasks_picker_empty_overlay_has_no_selectable_row() {
        let mut picker = TasksPickerState::new(Vec::new(), Vec::new());

        assert_eq!(picker.filtered_len(), 2, "both section headers remain");
        assert!(matches!(picker.selected_row(), Some(TasksRow::Header(_))));
        assert!(picker.selected_task().is_none());
        // Navigation is a no-op without selectable rows.
        picker.select_next();
        picker.select_prev();
        assert!(matches!(picker.selected_row(), Some(TasksRow::Header(_))));
    }

    #[test]
    fn test_tasks_picker_replace_rows_clamps_selection() {
        let mut picker = TasksPickerState::new(
            Vec::new(),
            vec![
                task("proc-1", "running", "sleep 60"),
                task("proc-2", "running", "make build"),
                task("proc-3", "running", "cargo test"),
            ],
        );
        // Move to the last task row.
        picker.select_next();
        picker.select_next();
        assert!(matches!(
            picker.selected_row(),
            Some(TasksRow::Task(ref task)) if task.process_id == "proc-3"
        ));

        // The refreshed list lost two tasks; the cursor clamps into range.
        picker.replace_rows(Vec::new(), vec![task("proc-1", "running", "sleep 60")]);
        assert!(matches!(
            picker.selected_row(),
            Some(TasksRow::Task(ref task)) if task.process_id == "proc-1"
        ));
    }

    #[test]
    fn test_tasks_picker_replace_rows_drops_preview_of_vanished_task() {
        let mut picker = TasksPickerState::new(
            Vec::new(),
            vec![
                task("proc-1", "running", "sleep 60"),
                task("proc-2", "running", "make build"),
            ],
        );
        picker.set_preview(&snapshot("proc-2", "out", ""));
        assert_eq!(picker.preview_process_id(), Some("proc-2"));

        // Task still listed: preview survives the refresh.
        picker.replace_rows(
            Vec::new(),
            vec![
                task("proc-1", "running", "sleep 60"),
                task("proc-2", "completed", "make build"),
            ],
        );
        assert_eq!(picker.preview_process_id(), Some("proc-2"));

        // Task gone: preview closes.
        picker.replace_rows(Vec::new(), vec![task("proc-1", "running", "sleep 60")]);
        assert!(picker.preview().is_none());
    }

    #[test]
    fn test_kill_effect_routes_by_row_kind() {
        let task_row = TasksRow::Task(task("proc-1", "running", "sleep 60"));
        assert_eq!(kill_effect(Some(&task_row)), KillEffect::KillTask("proc-1"));

        let subagent_row = TasksRow::Subagent(delegation("d-1", "researcher"));
        assert_eq!(kill_effect(Some(&subagent_row)), KillEffect::NotKillable);

        let header_row = TasksRow::Header("Subagents (1)".to_string());
        assert_eq!(kill_effect(Some(&header_row)), KillEffect::NotKillable);

        assert_eq!(kill_effect(None), KillEffect::Noop);
    }

    #[test]
    fn test_format_elapsed_compacts_durations() {
        assert_eq!(format_elapsed(Duration::from_secs(0)), "0s");
        assert_eq!(format_elapsed(Duration::from_secs(5)), "5s");
        assert_eq!(format_elapsed(Duration::from_secs(59)), "59s");
        assert_eq!(format_elapsed(Duration::from_secs(65)), "1m05s");
        assert_eq!(format_elapsed(Duration::from_secs(599)), "9m59s");
        assert_eq!(format_elapsed(Duration::from_secs(3600)), "1h00m");
        assert_eq!(format_elapsed(Duration::from_secs(3725)), "1h02m");
    }

    #[test]
    fn test_task_status_label_carries_finish_details() {
        let mut running = task("proc-1", "running", "sleep 60");
        running.duration_ms = 5_000;
        assert_eq!(task_status_label(&running), "running 5s");

        let mut completed = task("proc-2", "completed", "make build");
        completed.exit_code = Some(0);
        assert_eq!(task_status_label(&completed), "exit 0");

        let mut failed = task("proc-3", "failed", "make build");
        failed.error = Some("boom".into());
        assert_eq!(task_status_label(&failed), "failed: boom");

        let unknown = task("proc-4", "unknown", "make build");
        assert_eq!(task_status_label(&unknown), "unknown");
    }

    #[test]
    fn test_preview_lines_keeps_tail_and_marks_stderr() {
        let stdout = (1..=12)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = preview_lines(&snapshot("proc-1", &stdout, ""));
        assert_eq!(lines.len(), PREVIEW_MAX_LINES);
        assert_eq!(lines[0], "line 3", "only the last 10 lines survive");
        assert_eq!(lines[9], "line 12");

        let with_stderr = preview_lines(&snapshot("proc-1", "out\n", "boom\n"));
        assert_eq!(with_stderr, vec!["out", "-- stderr --", "boom"]);

        assert_eq!(
            preview_lines(&snapshot("proc-1", "", "")),
            vec!["(no output)"]
        );
    }

    #[test]
    fn test_preview_toggle_state() {
        let mut picker =
            TasksPickerState::new(Vec::new(), vec![task("proc-1", "running", "sleep")]);
        assert!(picker.preview().is_none());

        picker.set_preview(&snapshot("proc-1", "hello\n", ""));
        assert_eq!(picker.preview_process_id(), Some("proc-1"));

        picker.clear_preview();
        assert!(picker.preview().is_none());
    }
}
