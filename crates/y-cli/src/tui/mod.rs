//! TUI application shell -- entry point, terminal setup, main event loop.
//!
//! `TuiApp` manages the ratatui terminal lifecycle (raw mode, alternate screen)
//! and drives the render-event-update loop. It delegates rendering to panel
//! modules and key handling to the key dispatcher (both in Phase T3+).

pub mod chat_flow;
pub mod clipboard;
pub mod commands;
pub mod events;
pub mod keys;
pub mod layout;
pub mod markdown;
pub mod overlays;
pub mod panels;
pub mod selection;
pub mod state;
pub mod theme;
pub mod tool_renderers;
pub mod tracing_bridge;

use std::fmt::Write as _;
use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Alignment;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use tracing::warn;
use tui_textarea::{CursorMove, TextArea};

use crate::wire::AppServices;
use chat_flow::{ActiveChat, InputIntent};
use commands::handlers::{self, AsyncCommand, CommandResult};
use events::{AppEvent, EventLoop};
use keys::KeyAction;
use layout::LayoutChunks;
use overlays::command_palette::CommandPaletteState;
use overlays::copy_picker::CopyPickerState;
use overlays::prompt_picker::{PromptPickerSelection, PromptPickerState};
use overlays::queue_picker::QueuePickerState;
use overlays::session_picker::SessionPickerState;
use overlays::tasks_picker::{kill_effect, KillEffect, TasksPickerState};
use state::{
    AppState, ChatMessage, ChatRenderCache, InteractionMode, MessageRole, PanelFocus,
    PromptTemplateStatus, SessionListItem, Toast, ToastLevel, ToolSelection,
};
use y_core::provider::ProviderPool as _;

/// Type alias for the ratatui terminal with crossterm backend.
type Term = Terminal<CrosstermBackend<Stdout>>;

// ---------------------------------------------------------------------------
// TuiApp
// ---------------------------------------------------------------------------

/// The top-level TUI application.
///
/// Owns the terminal handle, application state, and event loop. The `run()`
/// method drives the main loop until the user quits.
pub struct TuiApp {
    /// Ratatui terminal handle.
    terminal: Term,
    /// Full application state.
    state: AppState,
    /// Async event loop (crossterm + tick).
    events: EventLoop,
    /// Input text area widget.
    textarea: TextArea<'static>,
    /// Command palette state (active in Command mode).
    palette: CommandPaletteState,
    /// Full-screen copy target selector state.
    copy_picker: CopyPickerState,
    /// Full-screen session resume selector state.
    session_picker: SessionPickerState,
    /// Full-screen session prompt-template selector state.
    prompt_picker: PromptPickerState,
    /// Follow-up queue overlay state.
    queue_picker: QueuePickerState,
    /// `/tasks` overlay state (subagents + background tasks).
    tasks_picker: TasksPickerState,
    /// Latest background-task list poll, driving the `bg: N` status-bar badge
    /// and the `/tasks` overlay rows.
    bg_tasks_cache: Vec<y_service::BackgroundTaskInfo>,
    /// Sender half of the background-task poll channel (cloned into the
    /// spawned poll task).
    bg_poll_tx:
        tokio::sync::mpsc::UnboundedSender<Result<Vec<y_service::BackgroundTaskInfo>, String>>,
    /// Receiver half of the background-task poll channel, drained on ticks.
    bg_poll_rx:
        tokio::sync::mpsc::UnboundedReceiver<Result<Vec<y_service::BackgroundTaskInfo>, String>>,
    /// Whether a background-task list poll is currently in flight; prevents
    /// overlapping spawns when a poll outlives its interval.
    bg_poll_in_flight: bool,
    /// Application services (LLM, session, etc.).
    services: Arc<AppServices>,
    /// Active service turn, including progress events and cancellation.
    active_chat: Option<ActiveChat>,
    /// Receiver for toast messages from the tracing bridge.
    toast_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Toast>>,
    /// Last computed layout chunks for mouse hit-testing.
    last_chunks: Option<LayoutChunks>,
    /// Lazily loaded cache of user prompt templates.
    ///
    /// Loaded once on first use; only successful loads are cached so a
    /// transient failure (e.g. a missing config directory) can still be
    /// retried on the next picker open.
    prompt_template_cache: Option<Vec<y_service::UserPromptTemplate>>,
    /// Cached plain-text lines from last chat render (for selection extraction).
    chat_plain_lines: Vec<String>,
    /// Cached tool-card row ranges from last chat render (for mouse hit-testing).
    chat_tool_rows: Vec<(std::ops::Range<usize>, ToolSelection)>,
    /// Per-message render cache for the chat panel (markdown/highlight/wrap).
    chat_render_cache: ChatRenderCache,
    /// Whether an input-composer mouse selection drag is in progress.
    selecting_input: bool,
    /// Composer viewport top row, replicated each frame from tui-textarea's
    /// internal scroll rule (the crate exposes no scroll-offset getter) so
    /// mouse clicks can be mapped to buffer positions.
    input_vscroll: u16,
    /// Whether the next loop iteration must redraw the frame.
    ///
    /// Set on any state-changing event (keys, mouse, resize, chat-stream
    /// batches, toast activity) and while animations are running. Idle ticks
    /// leave it cleared so the loop does not re-render an unchanged frame.
    needs_redraw: bool,
}

impl TuiApp {
    /// Create a new `TuiApp`, entering raw mode and alternate screen.
    ///
    /// `toast_rx` receives `Toast` values from the tracing bridge layer.
    /// Pass `None` if no tracing bridge is configured.
    pub fn new(
        services: Arc<AppServices>,
        toast_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Toast>>,
    ) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        let state = AppState::new();
        let events = EventLoop::new(Duration::from_millis(100));
        let textarea = TextArea::default();
        let palette = CommandPaletteState::new();
        let copy_picker = CopyPickerState::default();
        let session_picker = SessionPickerState::default();
        let prompt_picker = PromptPickerState::default();
        let queue_picker = QueuePickerState::default();
        let tasks_picker = TasksPickerState::default();
        let (bg_poll_tx, bg_poll_rx) = tokio::sync::mpsc::unbounded_channel();

        Ok(Self {
            terminal,
            state,
            events,
            textarea,
            palette,
            copy_picker,
            session_picker,
            prompt_picker,
            queue_picker,
            tasks_picker,
            bg_tasks_cache: Vec::new(),
            bg_poll_tx,
            bg_poll_rx,
            bg_poll_in_flight: false,
            services,
            active_chat: None,
            toast_rx,
            last_chunks: None,
            prompt_template_cache: None,
            chat_plain_lines: Vec::new(),
            chat_tool_rows: Vec::new(),
            chat_render_cache: ChatRenderCache::default(),
            selecting_input: false,
            input_vscroll: 0,
            needs_redraw: true,
        })
    }

    /// Resume a session by ID or ID prefix before entering the main loop.
    ///
    /// Called from `tui_cmd::run` when the user passes `--session` or uses
    /// the `resume` subcommand.
    pub async fn resume_session(&mut self, target: &str) {
        let workspace = match std::env::current_dir() {
            Ok(path) => path,
            Err(e) => {
                warn!(error = %e, "failed to resolve current workspace for resume");
                return;
            }
        };
        match y_service::SessionService::resolve_resume_target(
            &self.services.session_manager,
            &workspace,
            None,
            target,
        )
        .await
        {
            Ok(Some(node)) => {
                if let Err(error) = self.switch_active_session(&node.id).await {
                    self.state.push_toast(error, ToastLevel::Error);
                }
            }
            Ok(None) => warn!(
                target,
                "no session matching resume target in current workspace"
            ),
            Err(error) => warn!(%error, "failed to resolve workspace-scoped resume target"),
        }
    }

    /// Run the TUI main loop.
    ///
    /// Returns when the user presses `Ctrl+Q`, `Ctrl+D`, or `Ctrl+C`.
    /// Terminal cleanup (raw mode off, leave alternate screen) is guaranteed
    /// via the `restore_terminal` call in all exit paths.
    pub async fn run(&mut self) -> Result<()> {
        // Load session list at startup; the session itself is created lazily
        // on the first message (see `chat_flow::submit_message`).
        self.load_sessions().await;

        // Initialize context window and status-bar model from provider
        // metadata. The pool exposes no default-provider handle, so this
        // uses the first registered provider (routing order decides which
        // provider actually serves the first turn).
        if let Some(meta) = self.services.provider_pool().await.list_metadata().first() {
            self.state.context_window = meta.context_window;
            self.state.status_model.clone_from(&meta.model);
        }

        loop {
            // Redraw only when something changed since the last frame.
            if self.needs_redraw {
                self.draw()?;
                self.needs_redraw = false;
            }

            let Some(event) = self.events.next().await else {
                break;
            };

            if self.handle_app_event(event).await {
                break;
            }

            // Drain the chat-stream channel on every loop iteration so
            // streaming text appears immediately instead of waiting for the
            // next tick.
            if self.drain_chat_events() {
                self.needs_redraw = true;
            }
        }

        self.restore_terminal()?;
        Ok(())
    }

    /// Process one event-loop event plus any events already queued behind it.
    ///
    /// Batching collapses bursts (e.g. mouse drag streams) into a single
    /// state-update sequence followed by at most one redraw: consecutive
    /// left-drag events are coalesced so only the latest position is applied.
    /// Returns `true` when the app should quit.
    async fn handle_app_event(&mut self, event: AppEvent) -> bool {
        use crossterm::event::{MouseButton, MouseEventKind};

        let mut pending_drag: Option<crossterm::event::MouseEvent> = None;
        let mut event = event;
        loop {
            match event {
                AppEvent::Mouse(mouse)
                    if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left)) =>
                {
                    // Coalesce consecutive drag events: only the latest matters.
                    pending_drag = Some(mouse);
                }
                other => {
                    self.flush_pending_drag(&mut pending_drag);
                    match other {
                        AppEvent::Key(key) => {
                            if self.handle_key_event(key).await {
                                return true;
                            }
                            self.needs_redraw = true;
                        }
                        AppEvent::Mouse(mouse) => {
                            self.handle_mouse_event(mouse);
                            self.needs_redraw = true;
                        }
                        AppEvent::Paste(text) => {
                            self.handle_paste(&text);
                            self.needs_redraw = true;
                        }
                        AppEvent::Resize(_w, _h) => {
                            self.needs_redraw = true;
                        }
                        AppEvent::Tick => self.handle_tick(),
                    }
                }
            }

            // Pull events that are already queued so bursts are handled in
            // one pass. `timeout(ZERO, ...)` polls the receiver first, so it
            // completes immediately for queued events and times out only when
            // the queue is empty.
            match tokio::time::timeout(Duration::ZERO, self.events.next()).await {
                Ok(Some(next)) => event = next,
                Ok(None) | Err(_) => break,
            }
        }
        self.flush_pending_drag(&mut pending_drag);
        false
    }

    /// Apply a coalesced mouse drag event, if one is pending.
    fn flush_pending_drag(&mut self, pending: &mut Option<crossterm::event::MouseEvent>) {
        if let Some(mouse) = pending.take() {
            self.handle_mouse_event(mouse);
            self.needs_redraw = true;
        }
    }

    /// Drain pending chat-stream events from the active turn.
    ///
    /// Returns `true` when at least one event was applied or the turn ended
    /// (both require a redraw), `false` when the channel was empty.
    fn drain_chat_events(&mut self) -> bool {
        let Some(ref mut active_chat) = self.active_chat else {
            return false;
        };
        let mut applied = false;
        let mut channel_closed = false;
        loop {
            match active_chat.events.try_recv() {
                Ok(event) => {
                    chat_flow::apply_chat_event(event, &mut self.state);
                    applied = true;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    break;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    channel_closed = true;
                    break;
                }
            }
        }
        if channel_closed {
            handle_chat_channel_closed(&mut self.state);
            self.active_chat = None;
            applied = true;
        }
        applied
    }

    /// Process a key event. Returns `true` when the app should quit.
    async fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> bool {
        let action = keys::dispatch(key, &self.state);
        // Any keystroke ends an in-progress composer mouse selection: the
        // highlight must not survive edits, history recalls, or mode changes
        // (all of which are key-driven).
        self.cancel_input_selection();
        match action {
            KeyAction::Quit => return true,
            KeyAction::Submit => {
                if self.handle_submit().await {
                    return true;
                }
            }
            KeyAction::InputPassthrough => {
                self.handle_input_passthrough(key);
            }
            KeyAction::CycleFocus => {
                self.state.cycle_focus_forward();
            }
            KeyAction::ScrollUp => {
                if self.state.mode == InteractionMode::Resume {
                    self.session_picker.select_prev();
                } else if self.state.mode == InteractionMode::Copy {
                    self.copy_picker.select_prev();
                } else if self.state.mode == InteractionMode::Prompt {
                    self.prompt_picker.select_prev();
                } else if self.state.mode == InteractionMode::Queue {
                    self.queue_picker.select_prev();
                } else if self.state.mode == InteractionMode::Tasks {
                    self.tasks_picker.select_prev();
                } else if self.state.mode == InteractionMode::Command {
                    self.palette.select_prev();
                } else {
                    self.state.scroll_offset = self.state.scroll_offset.saturating_add(3);
                }
            }
            KeyAction::ScrollDown => {
                if self.state.mode == InteractionMode::Resume {
                    self.session_picker.select_next();
                } else if self.state.mode == InteractionMode::Copy {
                    self.copy_picker.select_next();
                } else if self.state.mode == InteractionMode::Prompt {
                    self.prompt_picker.select_next();
                } else if self.state.mode == InteractionMode::Queue {
                    self.queue_picker.select_next();
                } else if self.state.mode == InteractionMode::Tasks {
                    self.tasks_picker.select_next();
                } else if self.state.mode == InteractionMode::Command {
                    self.palette.select_next();
                } else {
                    self.state.scroll_offset = self.state.scroll_offset.saturating_sub(3);
                }
            }
            KeyAction::PageScrollUp => {
                if self.state.mode == InteractionMode::Resume {
                    for _ in 0..10 {
                        self.session_picker.select_prev();
                    }
                } else if self.state.mode == InteractionMode::Copy {
                    for _ in 0..10 {
                        self.copy_picker.select_prev();
                    }
                } else if self.state.mode == InteractionMode::Prompt {
                    for _ in 0..10 {
                        self.prompt_picker.select_prev();
                    }
                } else {
                    let page = self.state.page_height.max(1);
                    self.state.scroll_offset = self.state.scroll_offset.saturating_add(page);
                }
            }
            KeyAction::PageScrollDown => {
                if self.state.mode == InteractionMode::Resume {
                    for _ in 0..10 {
                        self.session_picker.select_next();
                    }
                } else if self.state.mode == InteractionMode::Copy {
                    for _ in 0..10 {
                        self.copy_picker.select_next();
                    }
                } else if self.state.mode == InteractionMode::Prompt {
                    for _ in 0..10 {
                        self.prompt_picker.select_next();
                    }
                } else {
                    let page = self.state.page_height.max(1);
                    self.state.scroll_offset = self.state.scroll_offset.saturating_sub(page);
                }
            }
            KeyAction::ScrollToTop => {
                // Jump to the first line: an offset of (total - page) pins
                // the viewport to the top. The renderer saturates offsets
                // beyond this against the real content height each frame, so
                // a slightly stale line count still lands at the top.
                self.state.scroll_offset = self
                    .chat_plain_lines
                    .len()
                    .saturating_sub(self.state.page_height);
            }
            KeyAction::ScrollToBottom => {
                self.state.scroll_offset = 0;
            }
            KeyAction::CancelStreaming => {
                if let Some(active_chat) = &self.active_chat {
                    active_chat.cancel();
                    chat_flow::cancel_streaming(&mut self.state);
                    self.state
                        .push_toast("Cancelling response...".into(), ToastLevel::Info);
                }
            }
            KeyAction::ShowHelp => {
                if self.state.mode == InteractionMode::Help {
                    self.state.clear_backtrack_selection();
                    self.state.set_mode(InteractionMode::Normal);
                } else {
                    // Close any other overlay first, then show help.
                    if self.state.mode != InteractionMode::Normal {
                        self.state.clear_backtrack_selection();
                        self.state.set_mode(InteractionMode::Normal);
                    }
                    self.state.set_mode(InteractionMode::Help);
                }
            }
            KeyAction::EnterCommandMode => {
                if textarea_is_empty(&self.textarea) {
                    self.state.set_mode(InteractionMode::Command);
                    self.palette = CommandPaletteState::new();
                    self.copy_picker = CopyPickerState::default();
                    self.session_picker = SessionPickerState::default();
                    self.prompt_picker = PromptPickerState::default();
                    self.queue_picker = QueuePickerState::default();
                    self.tasks_picker = TasksPickerState::default();
                } else {
                    // ':' inside a non-empty draft (e.g. "12:30", URLs) is
                    // literal text, not a command-mode trigger.
                    self.handle_input_passthrough(key);
                }
            }
            KeyAction::EnterBacktrack => {
                self.open_backtrack_picker();
            }
            KeyAction::BacktrackPrevious => {
                self.state.select_previous_user_message();
            }
            KeyAction::BacktrackNext => {
                self.state.select_next_user_message();
            }
            KeyAction::ConfirmBacktrack => {
                self.confirm_backtrack_selection().await;
            }
            KeyAction::ReturnToNormal => {
                self.state.clear_backtrack_selection();
                self.state.set_mode(InteractionMode::Normal);
                self.state.set_focus(PanelFocus::Input);
                self.palette = CommandPaletteState::new();
            }
            KeyAction::HistoryPrev => {
                let cursor_row = self.textarea.cursor().0;
                let line_count = self.textarea.lines().len();
                if line_count > 1 && cursor_row > 0 {
                    // Multi-line with cursor not at first line: move within text.
                    self.textarea.input(key);
                } else {
                    // Save draft if entering history for the first time.
                    if self.state.history_index.is_none() {
                        let current = self.textarea.lines().join("\n");
                        self.state.input_draft = Some(current);
                    }
                    if let Some(entry) = self.state.history_prev() {
                        self.textarea = TextArea::new(vec![entry.to_string()]);
                    }
                }
            }
            KeyAction::HistoryNext => {
                let cursor_row = self.textarea.cursor().0;
                let line_count = self.textarea.lines().len();
                let last_line = line_count.saturating_sub(1);
                if line_count > 1 && cursor_row < last_line {
                    // Multi-line with cursor not at last line: move within text.
                    self.textarea.input(key);
                } else {
                    match self.state.history_next() {
                        Some(entry) => {
                            self.textarea = TextArea::new(vec![entry.to_string()]);
                        }
                        None => {
                            // Restore draft if available.
                            if let Some(draft) = self.state.input_draft.take() {
                                if draft.is_empty() {
                                    self.textarea = TextArea::default();
                                } else {
                                    let lines: Vec<String> =
                                        draft.split('\n').map(String::from).collect();
                                    self.textarea = TextArea::new(lines);
                                }
                            } else {
                                self.textarea = TextArea::default();
                            }
                        }
                    }
                }
            }
            action @ (KeyAction::SelectNextTool
            | KeyAction::SelectPreviousTool
            | KeyAction::ToggleSelectedTool
            | KeyAction::CopySelectedTool) => self.handle_tool_action(action),
            KeyAction::QueueDelete => {
                self.queue_delete_selected();
            }
            KeyAction::QueueSteer => {
                self.queue_toggle_steer_selected().await;
            }
            KeyAction::TasksKill => {
                self.tasks_kill_selected().await;
            }
            KeyAction::TasksRefresh => {
                self.tasks_refresh().await;
            }
            KeyAction::Consumed | KeyAction::Unhandled => {}
        }
        false
    }

    /// Handle `KeyAction::Submit` for the active interaction mode.
    ///
    /// Picker overlays confirm their selection, Command mode executes the
    /// palette input, and Normal mode classifies the composer text (new turn,
    /// slash command, or queued follow-up). Returns `true` when the executed
    /// command requested quitting the app.
    async fn handle_submit(&mut self) -> bool {
        if self.state.mode == InteractionMode::Resume {
            let Some(session_id) = self
                .session_picker
                .selected_session()
                .map(|session| session.id.clone())
            else {
                self.state
                    .push_toast("No session selected.".into(), ToastLevel::Info);
                return false;
            };
            self.cmd_switch_session(&session_id).await;
            self.state.set_mode(InteractionMode::Normal);
            self.state.set_focus(PanelFocus::Input);
        } else if self.state.mode == InteractionMode::Copy {
            let Some(item) = self.copy_picker.selected_item().cloned() else {
                self.state
                    .push_toast("No copy target selected.".into(), ToastLevel::Info);
                return false;
            };
            self.deliver_copy(&item.content, &item.label);
            self.state.set_mode(InteractionMode::Normal);
            self.state.set_focus(PanelFocus::Input);
        } else if self.state.mode == InteractionMode::Prompt {
            let Some(selection) = self.prompt_picker.selected_choice() else {
                self.state
                    .push_toast("No prompt template selected.".into(), ToastLevel::Info);
                return false;
            };
            self.apply_prompt_selection(selection).await;
        } else if self.state.mode == InteractionMode::Queue {
            // Enter closes the overlay; queue mutations use d/s.
            self.state.set_mode(InteractionMode::Normal);
            self.state.set_focus(PanelFocus::Input);
        } else if self.state.mode == InteractionMode::Tasks {
            // Enter toggles the selected task's inline output preview.
            self.tasks_toggle_preview().await;
        } else if self.state.mode == InteractionMode::Command {
            if self.palette.in_arg_mode() {
                let cmd = self.palette.arg_command.clone().unwrap_or_default();
                let arg = self
                    .palette
                    .selected_arg()
                    .map_or_else(|| self.palette.input.trim().to_string(), str::to_string);
                let cmd_input = if arg.is_empty() {
                    cmd
                } else {
                    format!("{cmd} {arg}")
                };
                if self.execute_command(&cmd_input).await {
                    return true;
                }
                self.palette = CommandPaletteState::new();
                self.state.set_mode(InteractionMode::Normal);
                self.state.set_focus(PanelFocus::Input);
            } else {
                let cmd_input = if let Some(selected) = self.palette.selected_command() {
                    selected.to_string()
                } else {
                    self.palette.input.clone()
                };
                if self.should_enter_arg_mode(&cmd_input).await {
                    // Stay in command mode with arg completions.
                } else {
                    if self.execute_command(&cmd_input).await {
                        return true;
                    }
                    self.palette = CommandPaletteState::new();
                    self.state.set_mode(InteractionMode::Normal);
                    self.state.set_focus(PanelFocus::Input);
                }
            }
        } else {
            let input: String = self.textarea.lines().join("\n");
            let clear_input = match chat_flow::classify_input(&input, self.state.is_streaming) {
                InputIntent::Ignore => false,
                InputIntent::Command(command) => {
                    self.state.push_history(input.trim());
                    if self.execute_command(&command).await {
                        return true;
                    }
                    true
                }
                InputIntent::NewTurn(text) => {
                    self.state.push_history(&text);
                    self.active_chat =
                        chat_flow::submit_message(&text, &mut self.state, &self.services);
                    true
                }
                InputIntent::FollowUp(text) => {
                    match chat_flow::enqueue_follow_up(&text, &self.state, &self.services) {
                        Ok(_) => {
                            self.state.push_history(&text);
                            chat_flow::refresh_follow_up_queue(&mut self.state, &self.services);
                            self.state.push_toast(
                                "Follow-up queued for the active run.".into(),
                                ToastLevel::Success,
                            );
                            true
                        }
                        Err(error) => {
                            self.state.push_toast(
                                format!("Could not queue follow-up: {error}"),
                                ToastLevel::Error,
                            );
                            false
                        }
                    }
                }
            };
            if clear_input {
                self.textarea = TextArea::default();
            }
        }
        false
    }

    fn handle_input_passthrough(&mut self, key: crossterm::event::KeyEvent) {
        if self.state.mode == InteractionMode::Resume {
            if let crossterm::event::KeyCode::Char(character) = key.code {
                self.session_picker.push_char(character);
            } else if key.code == crossterm::event::KeyCode::Backspace {
                self.session_picker.pop_char();
            }
        } else if self.state.mode == InteractionMode::Copy {
            if let crossterm::event::KeyCode::Char(character) = key.code {
                self.copy_picker.push_char(character);
            } else if key.code == crossterm::event::KeyCode::Backspace {
                self.copy_picker.pop_char();
            }
        } else if self.state.mode == InteractionMode::Prompt {
            if let crossterm::event::KeyCode::Char(character) = key.code {
                self.prompt_picker.push_char(character);
            } else if key.code == crossterm::event::KeyCode::Backspace {
                self.prompt_picker.pop_char();
            }
        } else if self.state.mode == InteractionMode::Command {
            if let crossterm::event::KeyCode::Char(character) = key.code {
                self.palette.push_char(character);
            } else if key.code == crossterm::event::KeyCode::Backspace {
                if self.palette.in_arg_mode() && self.palette.input.is_empty() {
                    self.palette = CommandPaletteState::new();
                } else {
                    self.palette.pop_char();
                }
            }
        } else if key.code == crossterm::event::KeyCode::Char('/')
            && textarea_is_empty(&self.textarea)
        {
            self.state.set_mode(InteractionMode::Command);
            self.palette = CommandPaletteState::new();
        } else {
            self.textarea.input(key);
        }
    }

    fn handle_paste(&mut self, text: &str) {
        // A paste edits the composer like typed input: end any in-progress
        // mouse selection first so the highlight cannot go stale.
        self.cancel_input_selection();
        match self.state.mode {
            // Picker modes feed their single-line filter input, mirroring
            // `handle_input_passthrough`; line breaks are dropped.
            InteractionMode::Resume => {
                for character in single_line_paste_text(text).chars() {
                    self.session_picker.push_char(character);
                }
            }
            InteractionMode::Copy => {
                for character in single_line_paste_text(text).chars() {
                    self.copy_picker.push_char(character);
                }
            }
            InteractionMode::Prompt => {
                for character in single_line_paste_text(text).chars() {
                    self.prompt_picker.push_char(character);
                }
            }
            InteractionMode::Command => {
                for character in single_line_paste_text(text).chars() {
                    self.palette.push_char(character);
                }
            }
            // Normal-ish modes insert into the composer. Unlike typed input,
            // a paste never triggers mode switches: a leading '/' on an
            // empty draft stays literal text.
            _ => {
                self.textarea.insert_str(text);
            }
        }
    }

    fn handle_tool_action(&mut self, action: KeyAction) {
        match action {
            KeyAction::SelectNextTool => {
                if self.state.select_next_tool().is_none() {
                    self.state.push_toast(
                        "No tool calls in this conversation.".into(),
                        ToastLevel::Info,
                    );
                }
            }
            KeyAction::SelectPreviousTool => {
                if self.state.select_previous_tool().is_none() {
                    self.state.push_toast(
                        "No tool calls in this conversation.".into(),
                        ToastLevel::Info,
                    );
                }
            }
            KeyAction::ToggleSelectedTool => {
                // With no active selection the toggle targets the most recent
                // tool card (the one the user is watching); the toast only
                // fires when the transcript has no tool cards at all.
                if !toggle_tool_display(&mut self.state) {
                    self.state.push_toast(
                        "No tool calls in this conversation.".into(),
                        ToastLevel::Info,
                    );
                }
            }
            KeyAction::CopySelectedTool => {
                let Some(tool) = self.state.selected_tool().cloned() else {
                    self.state.push_toast(
                        "Select a tool card with [ or ] first.".into(),
                        ToastLevel::Info,
                    );
                    return;
                };
                let content = commands::copy::format_tool_call_for_copy(&tool);
                self.deliver_copy(&content, &format!("{} tool call", tool.name));
            }
            _ => {}
        }
    }

    /// Cancel an in-progress composer mouse selection, if any.
    fn cancel_input_selection(&mut self) {
        if self.selecting_input {
            self.textarea.cancel_selection();
            self.selecting_input = false;
        }
    }

    /// Process a mouse event.
    fn handle_mouse_event(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        if matches!(
            self.state.mode,
            InteractionMode::Copy
                | InteractionMode::Resume
                | InteractionMode::Select
                | InteractionMode::Prompt
                | InteractionMode::Queue
                | InteractionMode::Tasks
        ) {
            return;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(chunks) = self.last_chunks.clone() else {
                    return;
                };
                let (x, y) = (mouse.column, mouse.row);
                if contains(chunks.input, x, y) {
                    self.state.set_focus(PanelFocus::Input);
                    self.state.selection.reset();
                    // Begin a composer text selection at the click position
                    // (`start_selection` resets any previous anchor).
                    let (row, col) = input_buffer_position(chunks.input, self.input_vscroll, x, y);
                    self.textarea.move_cursor(CursorMove::Jump(row, col));
                    self.textarea.start_selection();
                    self.selecting_input = true;
                } else {
                    // A click outside the composer ends its selection.
                    self.cancel_input_selection();
                    if contains(chunks.chat, x, y) {
                        self.state.set_focus(PanelFocus::Chat);
                        let (row, col) = self.terminal_to_content(x, y, chunks.chat);
                        self.state.selection.start(row, col);
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.selecting_input {
                    // Extend the composer selection; `Jump` clamps positions
                    // outside the panel to the buffer edges.
                    if let Some(ref chunks) = self.last_chunks {
                        let (row, col) = input_buffer_position(
                            chunks.input,
                            self.input_vscroll,
                            mouse.column,
                            mouse.row,
                        );
                        self.textarea.move_cursor(CursorMove::Jump(row, col));
                    }
                } else if self.state.selection.active {
                    if let Some(ref chunks) = self.last_chunks {
                        let (row, col) =
                            self.terminal_to_content(mouse.column, mouse.row, chunks.chat);
                        self.state.selection.update(row, col);
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.selecting_input {
                    self.selecting_input = false;
                    if let Some(ref chunks) = self.last_chunks {
                        let (row, col) = input_buffer_position(
                            chunks.input,
                            self.input_vscroll,
                            mouse.column,
                            mouse.row,
                        );
                        self.textarea.move_cursor(CursorMove::Jump(row, col));
                    }
                    // A pure click yanks an empty string; only a real span
                    // reaches the clipboard.
                    if self.textarea.is_selecting() {
                        self.textarea.copy();
                        self.textarea.cancel_selection();
                        let text = self.textarea.yank_text();
                        if !text.is_empty() {
                            self.deliver_copy(&text, "input");
                        }
                    }
                } else if self.state.selection.active {
                    if let Some(ref chunks) = self.last_chunks {
                        let (row, col) =
                            self.terminal_to_content(mouse.column, mouse.row, chunks.chat);
                        self.state.selection.update(row, col);
                    }
                    self.state.selection.finish();

                    if !self.state.selection.is_empty() {
                        // Drag selection: copy the highlighted text.
                        let text =
                            selection::extract_text(&self.chat_plain_lines, &self.state.selection);
                        if !text.is_empty() {
                            self.deliver_copy(&text, "selection");
                        }
                    } else if let Some(ref chunks) = self.last_chunks {
                        // Pure click (no drag): toggle the tool card under
                        // the cursor, if any.
                        if contains(chunks.chat, mouse.column, mouse.row) {
                            let (row, _) =
                                self.terminal_to_content(mouse.column, mouse.row, chunks.chat);
                            if let Some(tool) = tool_at_row(&self.chat_tool_rows, row) {
                                self.state.selected_tool = Some(tool);
                                self.state.cycle_selected_tool_display();
                            }
                        }
                    }
                }
            }
            MouseEventKind::Down(_) => {
                self.state.selection.reset();
                self.cancel_input_selection();
            }
            MouseEventKind::ScrollUp => {
                self.state.scroll_offset = self.state.scroll_offset.saturating_add(3);
            }
            MouseEventKind::ScrollDown => {
                self.state.scroll_offset = self.state.scroll_offset.saturating_sub(3);
            }
            _ => {}
        }
    }

    /// Handle periodic tick: update animations and toasts, drain channels.
    fn handle_tick(&mut self) {
        self.state.tick_animation();

        let mut toasts_changed = false;
        if let Some(ref mut rx) = self.toast_rx {
            while let Ok(toast) = rx.try_recv() {
                self.state.push_toast(toast.message, toast.level);
                toasts_changed = true;
            }
        }

        let toasts_before = self.state.toasts.len();
        self.state.tick_toasts();
        toasts_changed = toasts_changed || self.state.toasts.len() != toasts_before;

        if self.drain_chat_events() {
            self.needs_redraw = true;
        }

        self.poll_background_activity();

        if tick_marks_dirty(
            self.state.is_streaming,
            !self.state.toasts.is_empty(),
            toasts_changed,
        ) {
            self.needs_redraw = true;
        }
    }

    /// Refresh the background-task and subagent projections.
    ///
    /// The subagent count comes from the in-memory delegation tracker and is
    /// cheap enough to read on every tick. The background-task list needs an
    /// async service call, so it is throttled (~1.5 s) and spawned off the
    /// UI loop; the result arrives through a channel drained on later ticks.
    fn poll_background_activity(&mut self) {
        // Apply a finished poll first so counts update as soon as possible.
        while let Ok(result) = self.bg_poll_rx.try_recv() {
            self.bg_poll_in_flight = false;
            self.apply_bg_task_list(result);
        }

        let agent_count = self.services.delegation_tracker.active_delegations().len();
        if agent_count != self.state.active_subagent_count {
            self.state.active_subagent_count = agent_count;
            self.needs_redraw = true;
            if self.state.mode == InteractionMode::Tasks {
                self.repopulate_tasks_picker();
            }
        }

        let overlay_open = self.state.mode == InteractionMode::Tasks;
        if self.bg_poll_in_flight
            || !bg_poll_due(
                self.state.tick_counter,
                self.state.is_streaming,
                self.state.bg_task_count,
                self.state.active_subagent_count,
                overlay_open,
            )
        {
            return;
        }
        // Without a session there are no session-owned tasks to list; the
        // subagent count above still updates.
        let Some(session_id) = self.state.current_session_id.clone() else {
            return;
        };
        self.bg_poll_in_flight = true;
        let services = Arc::clone(&self.services);
        let tx = self.bg_poll_tx.clone();
        tokio::spawn(async move {
            let result = y_service::BackgroundTaskService::list(&services, session_id)
                .await
                .map_err(|error| error.to_string());
            // A send failure means the app is shutting down; drop the result.
            let _ = tx.send(result);
        });
    }

    /// Apply a finished background-task poll: cache the rows, update the
    /// badge count, and refresh the open `/tasks` overlay.
    fn apply_bg_task_list(&mut self, result: Result<Vec<y_service::BackgroundTaskInfo>, String>) {
        // On error keep the previous projection; the next poll retries.
        let Ok(tasks) = result else {
            return;
        };
        let running = tasks.iter().filter(|task| task.status == "running").count();
        self.bg_tasks_cache = tasks;
        if running != self.state.bg_task_count {
            self.state.bg_task_count = running;
            self.needs_redraw = true;
        }
        if self.state.mode == InteractionMode::Tasks {
            self.repopulate_tasks_picker();
            self.needs_redraw = true;
        }
    }

    /// Draw the current frame.
    fn draw(&mut self) -> Result<()> {
        // Pre-compute input height and layout outside the closure so we
        // can store them for mouse hit-testing.
        let input_lines = panels::input::input_height(&self.textarea);
        let term_size = self.terminal.size()?;
        let term_rect = ratatui::layout::Rect::new(0, 0, term_size.width, term_size.height);
        let chunks = layout::compute_layout(term_rect, input_lines);

        // Update page_height from the chat panel for page-scroll calculations.
        self.state.page_height = chunks.chat.height.saturating_sub(2) as usize;

        // Store the layout for mouse hit-testing, then borrow it back (this
        // avoids cloning the chunks on every frame).
        self.last_chunks = Some(chunks);

        let state = &self.state;
        let textarea = &mut self.textarea;
        // Re-applied every frame: history recall and submit replace the
        // textarea, dropping per-instance styles like this one.
        textarea.set_selection_style(input_selection_style());
        let palette = &self.palette;
        let render_cache = &mut self.chat_render_cache;
        let plain_lines = &mut self.chat_plain_lines;
        let tool_rows = &mut self.chat_tool_rows;
        let Some(chunks_ref) = self.last_chunks.as_ref() else {
            unreachable!("layout stored above");
        };

        self.terminal.draw(|frame| {
            let area = frame.area();

            // Check minimum terminal size.
            if layout::is_terminal_too_small(area.width, area.height) {
                let msg = Paragraph::new(vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        "Terminal too small",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        format!(
                            "Minimum: {}x{} -- Current: {}x{}",
                            layout::MIN_COLS,
                            layout::MIN_ROWS,
                            area.width,
                            area.height
                        ),
                        Style::default().fg(Color::DarkGray),
                    )),
                ])
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Red)),
                );
                frame.render_widget(msg, area);
                return;
            }

            let chunks = chunks_ref;

            Self::render_panels(
                frame,
                chunks,
                state,
                textarea,
                render_cache,
                plain_lines,
                tool_rows,
            );

            // Render command palette overlay if in Command mode.
            if state.mode == InteractionMode::Command {
                overlays::command_palette::render(frame, area, palette, &state.theme);
            }

            if state.mode == InteractionMode::Copy {
                overlays::copy_picker::render(frame, area, &self.copy_picker, &state.theme);
            }

            if state.mode == InteractionMode::Resume {
                overlays::session_picker::render(frame, area, &self.session_picker, &state.theme);
            }

            if state.mode == InteractionMode::Select {
                overlays::backtrack_picker::render(frame, area, state, &state.theme);
            }

            if state.mode == InteractionMode::Prompt {
                overlays::prompt_picker::render(frame, area, &self.prompt_picker, &state.theme);
            }

            if state.mode == InteractionMode::Queue {
                overlays::queue_picker::render(frame, area, &self.queue_picker, &state.theme);
            }

            if state.mode == InteractionMode::Tasks {
                overlays::tasks_picker::render(frame, area, &self.tasks_picker, &state.theme);
            }

            // Render help overlay if in Help mode.
            if state.mode == InteractionMode::Help {
                overlays::help::render(frame, area);
            }

            // Render toast overlay (always, non-modal).
            overlays::toast::render(frame, area, &state.toasts);
        })?;

        // Track the composer viewport's top row for mouse hit-mapping.
        // tui-textarea keeps its scroll offset private, so the widget's
        // scroll rule is replicated (`next_scroll_top`) from the cursor row
        // and the panel's inner height (border excluded).
        let input_inner_height = self
            .last_chunks
            .as_ref()
            .map_or(0, |chunks| chunks.input.height.saturating_sub(2));
        let cursor_row = u16::try_from(self.textarea.cursor().0).unwrap_or(u16::MAX);
        self.input_vscroll = next_scroll_top(self.input_vscroll, cursor_row, input_inner_height);

        Ok(())
    }

    /// Check if a command should enter argument-completion mode instead of
    /// executing immediately. Returns `true` if arg mode was entered.
    async fn should_enter_arg_mode(&mut self, cmd_name: &str) -> bool {
        let resolved = commands::registry::CommandRegistry::shared().resolve_alias(cmd_name);

        match resolved {
            "model" => {
                let pool = self.services.provider_pool().await;
                let metadata = pool.list_metadata();
                if metadata.is_empty() {
                    return false;
                }
                let completions: Vec<(String, String)> = metadata
                    .iter()
                    .map(|m| (m.id.as_str().to_string(), m.model.clone()))
                    .collect();
                self.palette.enter_arg_mode("model".into(), completions);
                true
            }
            "resume" => self.open_session_picker().await,
            "goal" => {
                self.palette.enter_arg_mode("goal".into(), Vec::new());
                true
            }
            "mode" => {
                self.palette.enter_arg_mode(
                    "mode".into(),
                    vec![
                        ("fast".into(), "Direct execution".into()),
                        (
                            "auto".into(),
                            "Automatically select fast, plan, or loop".into(),
                        ),
                        ("plan".into(), "Reviewed structured planning".into()),
                        ("loop".into(), "Iterative execution and self-review".into()),
                    ],
                );
                true
            }
            "copy" => {
                self.open_copy_picker();
                true
            }
            "queue" => {
                self.open_queue_overlay();
                true
            }
            "tasks" => {
                self.open_tasks_overlay().await;
                true
            }
            "prompt" => {
                if !self.open_prompt_picker() {
                    self.state.set_mode(InteractionMode::Normal);
                    self.state.set_focus(PanelFocus::Input);
                }
                true
            }
            _ => false,
        }
    }

    /// Execute a command and apply its result to state.
    /// Returns `true` if the app should quit.
    async fn execute_command(&mut self, cmd_input: &str) -> bool {
        let result = handlers::execute(cmd_input, &mut self.state);
        match result {
            CommandResult::Ok(Some(msg)) => {
                self.state.push_toast(msg, ToastLevel::Info);
            }
            CommandResult::Error(msg) => {
                self.state
                    .push_toast(format!("Error: {msg}"), ToastLevel::Error);
            }
            CommandResult::Quit => {
                return true;
            }
            CommandResult::Ok(None) => {}
            CommandResult::NewSession => {
                // State has been reset by the handler (messages cleared,
                // current_session_id set to None, user_message_count reset).
                // Actual session creation is deferred to first message.
                self.state
                    .push_toast("New session started.".into(), ToastLevel::Info);
            }
            CommandResult::Async(cmd) => {
                self.execute_async_command(cmd).await;
            }
            CommandResult::SubmitTurn { input, mode } => {
                if self.state.is_streaming {
                    self.state.push_toast(
                        "A response is active. Enter plain text to queue a follow-up, or press Esc to cancel."
                            .into(),
                        ToastLevel::Error,
                    );
                    return false;
                }
                self.state.turn_mode = mode;
                self.active_chat = chat_flow::submit_message_with_mode(
                    &input,
                    mode,
                    &mut self.state,
                    &self.services,
                );
            }
            CommandResult::SetTurnMode(mode) => {
                self.state.turn_mode = mode;
                self.state
                    .push_toast(format!("Turn mode: {}", mode.label()), ToastLevel::Success);
            }
            CommandResult::Copy(target) => self.copy_to_clipboard(target),
            CommandResult::OpenCopyPicker => self.open_copy_picker(),
            CommandResult::OpenQueueOverlay => self.open_queue_overlay(),
            CommandResult::OpenTasksOverlay => self.open_tasks_overlay().await,
        }
        false
    }

    /// Execute an async command that requires service access.
    async fn execute_async_command(&mut self, cmd: AsyncCommand) {
        match cmd {
            AsyncCommand::ListSessions => self.cmd_list_sessions().await,
            AsyncCommand::SwitchSession(target) => self.cmd_switch_session(&target).await,
            AsyncCommand::ResumeSession(target) => match target {
                Some(target) => self.cmd_switch_session(&target).await,
                None => {
                    self.open_session_picker().await;
                }
            },
            AsyncCommand::DeleteSession(target) => self.cmd_delete_session(&target).await,
            AsyncCommand::BranchSession(label) => self.cmd_branch_session(label).await,
            AsyncCommand::ExportSession(ref format) => {
                self.cmd_export_session(format.as_deref());
            }
            AsyncCommand::ShowStats => self.cmd_show_stats(),
            AsyncCommand::CompactContext => self.cmd_compact_context().await,
            AsyncCommand::ModelCommand(ref provider_id) => {
                self.cmd_model(provider_id.clone()).await;
            }
            AsyncCommand::ShowAgents => self.cmd_show_agents().await,
            AsyncCommand::PromptTemplate(target) => match target {
                Some(target) => self.apply_prompt_target(&target).await,
                None => {
                    self.open_prompt_picker();
                }
            },
        }
    }

    // -----------------------------------------------------------------------
    // Async command implementations
    // -----------------------------------------------------------------------

    /// Open a full-screen session picker populated from recent sessions.
    async fn open_session_picker(&mut self) -> bool {
        self.load_sessions().await;
        if self.state.sessions.is_empty() {
            self.state
                .push_toast("No sessions available to resume.".into(), ToastLevel::Info);
            return false;
        }

        self.session_picker = SessionPickerState::new(
            self.state.sessions.clone(),
            self.state.current_session_id.as_deref(),
        );
        self.state.set_mode(InteractionMode::Resume);
        self.state.set_focus(PanelFocus::Chat);
        true
    }

    /// Return the cached prompt templates, loading them from disk on first
    /// use. Only successful loads are cached, so a transient failure can be
    /// retried on the next call.
    fn prompt_templates(&mut self) -> Result<&[y_service::UserPromptTemplate], String> {
        if self.prompt_template_cache.is_none() {
            self.prompt_template_cache = Some(load_prompt_templates()?);
        }
        let Some(templates) = self.prompt_template_cache.as_deref() else {
            unreachable!("cache populated above");
        };
        Ok(templates)
    }

    fn open_prompt_picker(&mut self) -> bool {
        if self.state.is_streaming {
            self.state.push_toast(
                "Wait for the active response before changing the prompt template.".into(),
                ToastLevel::Warning,
            );
            return false;
        }

        let templates = match self.prompt_templates() {
            Ok(templates) => templates.to_vec(),
            Err(error) => {
                self.state.push_toast(error, ToastLevel::Error);
                return false;
            }
        };
        self.prompt_picker =
            PromptPickerState::new(templates, self.state.prompt_template_status.template_id());
        self.state.set_mode(InteractionMode::Prompt);
        self.state.set_focus(PanelFocus::Chat);
        true
    }

    async fn apply_prompt_target(&mut self, target: &str) {
        if matches!(
            target.trim().to_ascii_lowercase().as_str(),
            "default" | "clear"
        ) {
            self.apply_prompt_selection(PromptPickerSelection::Default)
                .await;
            return;
        }

        let templates = match self.prompt_templates() {
            Ok(templates) => templates.to_vec(),
            Err(error) => {
                self.state.push_toast(error, ToastLevel::Error);
                return;
            }
        };
        let target_lower = target.trim().to_ascii_lowercase();
        let Some(template) = templates.into_iter().find(|template| {
            template.id.to_ascii_lowercase() == target_lower
                || template.name.to_ascii_lowercase() == target_lower
        }) else {
            self.state.push_toast(
                format!("No prompt template matching '{target}'."),
                ToastLevel::Error,
            );
            return;
        };
        self.apply_prompt_selection(PromptPickerSelection::Template(template))
            .await;
    }

    async fn apply_prompt_selection(&mut self, selection: PromptPickerSelection) {
        if self.state.is_streaming {
            self.state.push_toast(
                "Wait for the active response before changing the prompt template.".into(),
                ToastLevel::Warning,
            );
            return;
        }

        match selection {
            PromptPickerSelection::Default => {
                if let Some(current_id) = self.state.current_session_id.clone() {
                    let session_id = y_core::types::SessionId::from_string(current_id);
                    if let Err(error) = y_service::PromptTemplateService::clear_session_config(
                        &self.services.session_manager,
                        &session_id,
                    )
                    .await
                    {
                        self.state.push_toast(
                            format!("Failed to clear prompt template: {error}"),
                            ToastLevel::Error,
                        );
                        return;
                    }
                }
                self.state.prompt_template_status = PromptTemplateStatus::Default;
                self.finish_prompt_selection("Using the default prompt.");
            }
            PromptPickerSelection::Template(template) => {
                let session_id = match self.ensure_prompt_session().await {
                    Ok(session_id) => session_id,
                    Err(error) => {
                        self.state.push_toast(error, ToastLevel::Error);
                        return;
                    }
                };
                if let Err(error) = y_service::PromptTemplateService::apply_template(
                    &self.services.session_manager,
                    &session_id,
                    &template,
                )
                .await
                {
                    self.state.push_toast(
                        format!("Failed to apply prompt template: {error}"),
                        ToastLevel::Error,
                    );
                    return;
                }
                let message = format!("Prompt template: {}", template.name);
                self.state.prompt_template_status = PromptTemplateStatus::Template {
                    id: template.id,
                    name: template.name,
                };
                self.finish_prompt_selection(&message);
            }
        }
    }

    async fn ensure_prompt_session(&mut self) -> Result<y_core::types::SessionId, String> {
        if let Some(current_id) = self.state.current_session_id.clone() {
            return Ok(y_core::types::SessionId::from_string(current_id));
        }

        let workspace = std::env::current_dir()
            .map_err(|error| format!("Failed to resolve current workspace: {error}"))?;
        let session = y_service::SessionService::create_session(
            &self.services.session_manager,
            y_core::session::CreateSessionOptions {
                parent_id: None,
                session_type: y_core::session::SessionType::Main,
                agent_id: None,
                title: Some("New Chat".into()),
            },
            &workspace,
        )
        .await
        .map_err(|error| format!("Failed to create session: {error}"))?;
        self.state.current_session_id = Some(session.id.to_string());
        self.load_sessions().await;
        Ok(session.id)
    }

    fn finish_prompt_selection(&mut self, message: &str) {
        self.state.set_mode(InteractionMode::Normal);
        self.state.set_focus(PanelFocus::Input);
        self.prompt_picker = PromptPickerState::default();
        self.state
            .push_toast(message.to_string(), ToastLevel::Success);
    }

    /// Open the Codex-style prompt backtrack selector for the active session.
    fn open_backtrack_picker(&mut self) {
        if self.state.current_session_id.is_none() {
            self.state
                .push_toast("No active session to backtrack.".into(), ToastLevel::Info);
            return;
        }
        if self.state.begin_backtrack_selection().is_none() {
            self.state
                .push_toast("No user prompts to backtrack to.".into(), ToastLevel::Info);
            return;
        }

        self.state.set_mode(InteractionMode::Select);
        self.state.set_focus(PanelFocus::Chat);
    }

    /// Branch before the selected prompt and restore it to the input editor.
    async fn confirm_backtrack_selection(&mut self) {
        let Some(message_index) = self.state.selected_message else {
            self.state
                .push_toast("No prompt selected.".into(), ToastLevel::Info);
            return;
        };
        let Some(prompt) = self
            .state
            .selected_user_message()
            .map(|message| message.content.clone())
        else {
            self.state.clear_backtrack_selection();
            self.state.set_mode(InteractionMode::Normal);
            self.state.set_focus(PanelFocus::Input);
            self.state.push_toast(
                "The selected prompt is no longer available.".into(),
                ToastLevel::Warning,
            );
            return;
        };
        let Some(current_id) = self.state.current_session_id.clone() else {
            self.state
                .push_toast("No active session to backtrack.".into(), ToastLevel::Error);
            return;
        };

        let session_id = y_core::types::SessionId::from_string(current_id);
        let title = backtrack_branch_title(&prompt);
        match y_service::SessionService::branch_before_message(
            &self.services.session_manager,
            &session_id,
            message_index,
            Some(title),
        )
        .await
        {
            Ok(branch) => {
                if let Err(error) = self.switch_active_session(&branch.id).await {
                    self.state.push_toast(error, ToastLevel::Error);
                    return;
                }
                self.load_sessions().await;
                self.textarea = if prompt.is_empty() {
                    TextArea::default()
                } else {
                    TextArea::new(prompt.split('\n').map(String::from).collect())
                };
                self.state.history_index = None;
                self.state.input_draft = None;
                self.state.clear_backtrack_selection();
                self.state.set_mode(InteractionMode::Normal);
                self.state.set_focus(PanelFocus::Input);
                self.state.push_toast(
                    "Created a branch before the selected prompt. Edit it and press Enter.".into(),
                    ToastLevel::Success,
                );
            }
            Err(error) => {
                self.state
                    .push_toast(format!("Backtrack failed: {error}"), ToastLevel::Error);
            }
        }
    }

    /// Resolve a copy target and place it on the system clipboard.
    fn copy_to_clipboard(&mut self, target: commands::copy::CopyTarget) {
        let text = match commands::copy::resolve_target(&self.state.messages, target) {
            Ok(text) => text,
            Err(message) => {
                self.state.push_toast(message, ToastLevel::Info);
                return;
            }
        };

        let label = match target {
            commands::copy::CopyTarget::AssistantResponse(nth) => {
                format!("assistant response {nth}")
            }
            commands::copy::CopyTarget::LastCodeBlock => "code block".to_string(),
            commands::copy::CopyTarget::Transcript => "transcript".to_string(),
        };
        self.deliver_copy(&text, &label);
    }

    fn open_copy_picker(&mut self) {
        let items = commands::copy::discover_copy_items(&self.state.messages);
        if items.is_empty() {
            self.state
                .push_toast("No conversation content to copy.".into(), ToastLevel::Info);
            return;
        }
        self.copy_picker = CopyPickerState::new(items);
        self.state.set_mode(InteractionMode::Copy);
        self.state.set_focus(PanelFocus::Chat);
    }

    /// `/queue` -- open the follow-up queue overlay for the active run.
    ///
    /// Refreshes the projection first so the overlay reflects the live
    /// service-side queue. An empty queue still opens (read-only view).
    fn open_queue_overlay(&mut self) {
        chat_flow::refresh_follow_up_queue(&mut self.state, &self.services);
        self.repopulate_queue_picker(0);
        self.state.set_mode(InteractionMode::Queue);
        self.state.set_focus(PanelFocus::Chat);
    }

    /// Rebuild the queue picker from the projected queue, keeping the cursor
    /// clamped to a valid row.
    fn repopulate_queue_picker(&mut self, selected: usize) {
        let last = self.state.follow_up_queue.len().saturating_sub(1);
        self.queue_picker = QueuePickerState::new(self.state.follow_up_queue.clone());
        self.queue_picker.set_selected(selected.min(last));
    }

    /// Session ID of the active chat, if one exists.
    fn active_session_id(&self) -> Option<y_core::types::SessionId> {
        self.state
            .current_session_id
            .as_ref()
            .map(|id| y_core::types::SessionId::from_string(id.clone()))
    }

    /// Refresh the queue projection and repopulate the open overlay after a
    /// queue-mutating action, preserving the cursor position.
    fn sync_queue_overlay(&mut self) {
        chat_flow::refresh_follow_up_queue(&mut self.state, &self.services);
        let selected = self.queue_picker.selected();
        self.repopulate_queue_picker(selected);
    }

    /// Remove the selected follow-up from the service-side queue.
    fn queue_delete_selected(&mut self) {
        let (Some(session_id), Some(item)) = (
            self.active_session_id(),
            self.queue_picker.selected_item().cloned(),
        ) else {
            return;
        };
        // The service refuses to delete steering items (they stay queued).
        if y_service::ChatService::delete_follow_up(&self.services, &session_id, &item.id) {
            self.sync_queue_overlay();
            self.state
                .push_toast("Follow-up removed.".into(), ToastLevel::Success);
        } else {
            self.state.push_toast(
                "Could not remove the follow-up; un-steer it first.".into(),
                ToastLevel::Error,
            );
        }
    }

    /// Promote the selected follow-up to the pending steer, or demote the
    /// pending steer back to a regular queued follow-up.
    async fn queue_toggle_steer_selected(&mut self) {
        let (Some(session_id), Some(item)) = (
            self.active_session_id(),
            self.queue_picker.selected_item().cloned(),
        ) else {
            return;
        };
        let result: Result<(), String> = match item.status {
            y_service::FollowUpStatus::Pending => {
                y_service::ChatService::steer_follow_up(&self.services, &session_id, &item.id)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
            y_service::FollowUpStatus::Steering => {
                y_service::ChatService::unsteer_follow_up(&self.services, &session_id, &item.id)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
        };
        match result {
            Ok(()) => {
                self.sync_queue_overlay();
                let message = match item.status {
                    y_service::FollowUpStatus::Pending => "Follow-up will steer the next step.",
                    y_service::FollowUpStatus::Steering => "Steer moved back to the queue.",
                };
                self.state.push_toast(message.into(), ToastLevel::Success);
            }
            Err(error) => {
                self.state.push_toast(
                    format!("Could not update steer: {error}"),
                    ToastLevel::Error,
                );
            }
        }
    }

    /// `/tasks` -- open the background task and subagent overlay.
    ///
    /// Refreshes the projections first so the overlay reflects live data,
    /// then builds a fresh picker (no stale cursor or preview).
    async fn open_tasks_overlay(&mut self) {
        self.refresh_tasks_data().await;
        let delegations = self.services.delegation_tracker.active_delegations();
        self.tasks_picker = TasksPickerState::new(delegations, self.bg_tasks_cache.clone());
        self.state.set_mode(InteractionMode::Tasks);
        self.state.set_focus(PanelFocus::Chat);
    }

    /// Fetch fresh background-task/subagent projections for the badge counts
    /// and the `/tasks` overlay.
    async fn refresh_tasks_data(&mut self) {
        if let Some(session_id) = self.state.current_session_id.clone() {
            let result = y_service::BackgroundTaskService::list(&self.services, session_id)
                .await
                .map_err(|error| error.to_string());
            self.apply_bg_task_list(result);
        }
        let agent_count = self.services.delegation_tracker.active_delegations().len();
        if agent_count != self.state.active_subagent_count {
            self.state.active_subagent_count = agent_count;
        }
    }

    /// Rebuild the `/tasks` overlay rows from the cached projections, keeping
    /// the cursor clamped to a selectable row.
    fn repopulate_tasks_picker(&mut self) {
        let delegations = self.services.delegation_tracker.active_delegations();
        self.tasks_picker
            .replace_rows(delegations, self.bg_tasks_cache.clone());
    }

    /// Re-fetch the `/tasks` overlay data and repopulate the picker.
    async fn tasks_refresh(&mut self) {
        self.refresh_tasks_data().await;
        self.repopulate_tasks_picker();
        self.needs_redraw = true;
    }

    /// Kill the background task under the `/tasks` overlay cursor. Subagent
    /// rows and section headers cannot be killed from here.
    async fn tasks_kill_selected(&mut self) {
        let process_id = match kill_effect(self.tasks_picker.selected_row()) {
            KillEffect::KillTask(process_id) => process_id.to_string(),
            KillEffect::NotKillable => {
                self.state.push_toast(
                    "Subagents cannot be stopped from here.".into(),
                    ToastLevel::Info,
                );
                return;
            }
            KillEffect::Noop => return,
        };
        let Some(session_id) = self.state.current_session_id.clone() else {
            return;
        };
        let request = y_service::BackgroundTaskPollRequest {
            session_id,
            process_id: process_id.clone(),
            yield_time_ms: None,
            max_output_bytes: None,
        };
        match y_service::BackgroundTaskService::kill(&self.services, request).await {
            Ok(snapshot) => {
                self.state.push_toast(
                    format!("Task {process_id} killed (status: {}).", snapshot.status),
                    ToastLevel::Success,
                );
            }
            Err(error) => {
                self.state.push_toast(
                    format!("Could not kill task {process_id}: {error}"),
                    ToastLevel::Error,
                );
            }
        }
        self.tasks_refresh().await;
    }

    /// Toggle the inline output preview for the task under the cursor.
    async fn tasks_toggle_preview(&mut self) {
        let Some(task) = self.tasks_picker.selected_task().cloned() else {
            return;
        };
        if self.tasks_picker.preview_process_id() == Some(task.process_id.as_str()) {
            self.tasks_picker.clear_preview();
            return;
        }
        let Some(session_id) = self.state.current_session_id.clone() else {
            return;
        };
        let request = y_service::BackgroundTaskPollRequest {
            session_id,
            process_id: task.process_id.clone(),
            yield_time_ms: None,
            max_output_bytes: None,
        };
        match y_service::BackgroundTaskService::poll(&self.services, request).await {
            Ok(snapshot) => self.tasks_picker.set_preview(&snapshot),
            Err(error) => self.state.push_toast(
                format!("Could not read output of {}: {error}", task.process_id),
                ToastLevel::Error,
            ),
        }
    }

    fn deliver_copy(&mut self, text: &str, label: &str) {
        match clipboard::copy_text(text) {
            Ok(clipboard::ClipboardDelivery::Native) => self
                .state
                .push_toast(format!("Copied {label}."), ToastLevel::Success),
            Ok(clipboard::ClipboardDelivery::Osc52) => self.state.push_toast(
                format!("Copied {label} through the terminal."),
                ToastLevel::Success,
            ),
            Ok(clipboard::ClipboardDelivery::FallbackFile(path)) => self.state.push_toast(
                format!(
                    "Clipboard unavailable; saved {label} to {}.",
                    path.display()
                ),
                ToastLevel::Warning,
            ),
            Err(error) => self
                .state
                .push_toast(format!("Copy failed: {error}"), ToastLevel::Error),
        }
    }

    /// `/list` -- list all sessions.
    async fn cmd_list_sessions(&mut self) {
        match self.workspace_sessions().await {
            Ok(nodes) => {
                if nodes.is_empty() {
                    self.state
                        .push_toast("No sessions found.".into(), ToastLevel::Info);
                    return;
                }
                let mut text = format!("Sessions ({}):\n\n", nodes.len());
                for n in &nodes {
                    let title = n.title.as_deref().unwrap_or("(untitled)");
                    let active =
                        if self.state.current_session_id.as_deref() == Some(&n.id.to_string()) {
                            " [*]"
                        } else {
                            ""
                        };
                    let _ = writeln!(
                        text,
                        "  {}  {}  ({} msgs){active}",
                        &n.id.to_string()[..8],
                        title,
                        n.message_count,
                    );
                }
                self.state.messages.push(ChatMessage::system(text));
            }
            Err(e) => {
                self.state
                    .push_toast(format!("Failed to list sessions: {e}"), ToastLevel::Error);
            }
        }
    }

    /// `/switch <target>` -- switch to another session by ID prefix or title.
    async fn cmd_switch_session(&mut self, target: &str) {
        // The service run owns the active session until it finishes. This
        // guard also covers the Resume overlay, which confirms through here.
        if self.state.is_streaming {
            self.state
                .push_toast(handlers::STREAMING_ACTIVE_MESSAGE.into(), ToastLevel::Error);
            return;
        }
        let nodes = match self.workspace_sessions().await {
            Ok(n) => n,
            Err(e) => {
                self.state
                    .push_toast(format!("Failed to list sessions: {e}"), ToastLevel::Error);
                return;
            }
        };

        let matched = find_session_by_target(&nodes, target);

        match matched {
            Some(node) => {
                let sid = node.id.clone();
                let title = node
                    .title
                    .clone()
                    .unwrap_or_else(|| sid.to_string()[..8].to_string());
                match self.switch_active_session(&sid).await {
                    Ok(()) => {
                        self.state.set_focus(PanelFocus::Input);
                        self.state
                            .push_toast(format!("Switched to: {title}"), ToastLevel::Info);
                    }
                    Err(error) => {
                        self.state.push_toast(error, ToastLevel::Error);
                    }
                }
            }
            None => {
                self.state.push_toast(
                    format!("No session matching '{target}'."),
                    ToastLevel::Error,
                );
            }
        }
    }

    /// `/delete <target>` -- delete a session by ID prefix.
    async fn cmd_delete_session(&mut self, target: &str) {
        use y_core::session::SessionFilter;

        // Deleting the active session mid-turn would orphan the running turn.
        if self.state.is_streaming {
            self.state
                .push_toast(handlers::STREAMING_ACTIVE_MESSAGE.into(), ToastLevel::Error);
            return;
        }

        let nodes = match self
            .services
            .session_manager
            .list_sessions(&SessionFilter::default())
            .await
        {
            Ok(n) => n,
            Err(e) => {
                self.state
                    .push_toast(format!("Failed to list sessions: {e}"), ToastLevel::Error);
                return;
            }
        };

        let matched = find_session_by_target(&nodes, target);

        match matched {
            Some(node) => {
                let sid = node.id.clone();
                let is_current = self.state.current_session_id.as_deref() == Some(&sid.to_string());

                match self.services.session_manager.delete_session(&sid).await {
                    Ok(()) => {
                        self.services.cleanup_session_state(&sid).await;
                        self.state.push_toast(
                            format!("Deleted session: {}", &sid.to_string()[..8]),
                            ToastLevel::Info,
                        );
                        // Refresh recent session completions and status labels.
                        self.load_sessions().await;
                        // If we deleted the current session, clear the chat.
                        if is_current {
                            self.state.messages.clear();
                            self.state.current_session_id = None;
                            self.state.user_message_count = 0;
                            self.state.prompt_template_status = PromptTemplateStatus::Default;
                        }
                    }
                    Err(e) => {
                        self.state
                            .push_toast(format!("Delete failed: {e}"), ToastLevel::Error);
                    }
                }
            }
            None => {
                self.state.push_toast(
                    format!("No session matching '{target}'."),
                    ToastLevel::Error,
                );
            }
        }
    }

    /// `/branch [label]` -- fork current session.
    async fn cmd_branch_session(&mut self, label: Option<String>) {
        let Some(ref current_id) = self.state.current_session_id else {
            self.state
                .push_toast("No active session to branch.".into(), ToastLevel::Error);
            return;
        };

        let sid = y_core::types::SessionId::from_string(current_id.clone());

        // Fork at the last message (full fork).
        match self
            .services
            .session_manager
            .fork_session(&sid, usize::MAX, label)
            .await
        {
            Ok(fork) => {
                let fork_id = fork.id.to_string();
                let fork_title = fork.title.unwrap_or_else(|| fork_id[..8].to_string());
                match self.switch_active_session(&fork.id).await {
                    Ok(()) => {
                        self.load_sessions().await;
                        self.state.set_focus(PanelFocus::Input);
                        self.state
                            .push_toast(format!("Branched: {fork_title}"), ToastLevel::Info);
                    }
                    Err(error) => {
                        self.state.push_toast(error, ToastLevel::Error);
                    }
                }
            }
            Err(e) => {
                self.state
                    .push_toast(format!("Branch failed: {e}"), ToastLevel::Error);
            }
        }
    }

    /// `/export [format]` -- export session transcript to clipboard.
    fn cmd_export_session(&mut self, format: Option<&str>) {
        if self.state.messages.is_empty() {
            self.state
                .push_toast("No messages to export.".into(), ToastLevel::Info);
            return;
        }

        let fmt = format.unwrap_or("md");
        let content = if fmt == "json" {
            let entries: Vec<serde_json::Value> = self
                .state
                .messages
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "role": format!("{:?}", m.role),
                        "content": m.content,
                        "timestamp": m.timestamp.to_rfc3339(),
                    })
                })
                .collect();
            serde_json::to_string_pretty(&entries).unwrap_or_default()
        } else {
            // Markdown format.
            let mut md = String::new();
            let _ = writeln!(md, "# Chat Export\n");
            for m in &self.state.messages {
                let role = match m.role {
                    MessageRole::User => "User",
                    MessageRole::Assistant => "Assistant",
                    MessageRole::System => "System",
                    MessageRole::Tool => "Tool",
                };
                let _ = writeln!(md, "## {role}\n\n{}\n", m.content);
            }
            md
        };

        let label = format!("{fmt} export ({} messages)", self.state.messages.len());
        self.deliver_copy(&content, &label);
    }

    /// `/stats` -- show token/cost statistics.
    fn cmd_show_stats(&mut self) {
        let mut text = String::from("Session Statistics:\n\n");
        let _ = writeln!(
            text,
            "  Input tokens (cumulative):  {}",
            self.state.cumulative_input_tokens
        );
        let _ = writeln!(
            text,
            "  Output tokens (cumulative): {}",
            self.state.cumulative_output_tokens
        );
        let _ = writeln!(
            text,
            "  Last turn input tokens:     {}",
            self.state.last_input_tokens
        );
        let _ = writeln!(
            text,
            "  Context usage:              {:.1}%",
            self.state.context_usage_percent()
        );
        let _ = writeln!(
            text,
            "  Context window:             {} tokens",
            self.state.context_window
        );
        if let Some(cost) = self.state.last_cost {
            let _ = writeln!(text, "  Last turn cost:             ${cost:.6}");
        }
        let _ = writeln!(
            text,
            "  Messages in view:           {}",
            self.state.messages.len()
        );
        let _ = writeln!(
            text,
            "  Turn count:                 {}",
            self.state.user_message_count
        );

        self.state.messages.push(ChatMessage::system(text));
    }

    /// `/compact` -- trigger manual context compaction.
    async fn cmd_compact_context(&mut self) {
        let Some(ref current_id) = self.state.current_session_id else {
            self.state
                .push_toast("No active session to compact.".into(), ToastLevel::Error);
            return;
        };

        let sid = y_core::types::SessionId::from_string(current_id.clone());
        self.state
            .push_toast("Compacting context...".into(), ToastLevel::Info);

        match crate::orchestrator::compact_context(&self.services, &sid).await {
            Ok(report) => {
                if report.compaction_triggered {
                    let msg = format!(
                        "Compacted {} messages, saved ~{} tokens.",
                        report.messages_compacted, report.compaction_tokens_saved,
                    );
                    self.state.push_toast(msg, ToastLevel::Success);
                    if !report.compaction_summary.is_empty() {
                        self.state.messages.push(ChatMessage::system(format!(
                            "[Context Compacted]\n\n{}",
                            report.compaction_summary
                        )));
                    }
                } else {
                    self.state
                        .push_toast("Nothing to compact.".into(), ToastLevel::Info);
                }
            }
            Err(e) => {
                self.state
                    .push_toast(format!("Compaction failed: {e}"), ToastLevel::Error);
            }
        }
    }

    /// `/model [provider-id]` -- list models or switch active provider.
    async fn cmd_model(&mut self, provider_id: Option<String>) {
        let pool = self.services.provider_pool().await;
        let metadata = pool.list_metadata();

        if metadata.is_empty() {
            self.state
                .push_toast("No providers configured.".into(), ToastLevel::Info);
            return;
        }

        match provider_id {
            None => {
                // List mode.
                let statuses = pool.provider_statuses().await;
                let selected = self.state.selected_provider_id.as_deref();

                let mut text = format!("Configured Models ({}):\n\n", metadata.len());
                for meta in &metadata {
                    let status = statuses.iter().find(|s| s.id == meta.id);
                    let frozen = status.is_some_and(|s| s.is_frozen);
                    let frozen_str = if frozen { " [FROZEN]" } else { "" };
                    let active = if selected == Some(meta.id.as_str()) {
                        " [*]"
                    } else {
                        ""
                    };
                    let _ = writeln!(
                        text,
                        "  {:<16} {:<24} ctx:{}k  {:?}{frozen_str}{active}",
                        meta.id.as_str(),
                        meta.model,
                        meta.context_window / 1000,
                        meta.provider_type,
                    );
                }
                text.push_str("\nUse /model <provider-id> to switch.");
                self.state.messages.push(ChatMessage::system(text));
            }
            Some(id) => {
                // Selection mode: prefix-match against provider IDs.
                let matched = metadata
                    .iter()
                    .find(|m| m.id.as_str() == id || m.id.as_str().starts_with(&id));
                match matched {
                    Some(meta) => {
                        let pid = meta.id.as_str().to_string();
                        self.state.selected_provider_id = Some(pid.clone());
                        self.state.status_model.clone_from(&meta.model);
                        self.state.context_window = meta.context_window;
                        self.state.push_toast(
                            format!("Model: {} ({})", meta.model, pid),
                            ToastLevel::Success,
                        );
                    }
                    None => {
                        self.state.push_toast(
                            format!("Unknown provider: '{id}'. Use /model to list."),
                            ToastLevel::Error,
                        );
                    }
                }
            }
        }
    }

    /// `/agent` -- list registered agents.
    async fn cmd_show_agents(&mut self) {
        let registry = self.services.agent_registry.lock().await;
        let agents = registry.list();

        if agents.is_empty() {
            self.state
                .push_toast("No agents registered.".into(), ToastLevel::Info);
            return;
        }

        let mut text = format!("Registered Agents ({}):\n\n", agents.len());
        for def in &agents {
            let callable = if def.user_callable { " [callable]" } else { "" };
            let _ = writeln!(
                text,
                "  {:<24} {:?}  {:?}{callable}",
                def.id, def.mode, def.trust_tier,
            );
        }

        self.state.messages.push(ChatMessage::system(text));
    }

    /// Render all panels into their layout chunks.
    ///
    /// The chat panel reuses `render_cache` across frames and writes its
    /// plain-text lines into `plain_lines` for selection extraction.
    fn render_panels(
        frame: &mut ratatui::Frame,
        chunks: &LayoutChunks,
        state: &AppState,
        textarea: &mut TextArea<'_>,
        render_cache: &mut ChatRenderCache,
        plain_lines: &mut Vec<String>,
        tool_rows: &mut Vec<(std::ops::Range<usize>, ToolSelection)>,
    ) {
        // Chat panel -- fills plain text lines for selection and tool-card
        // row ranges for mouse hit-testing.
        panels::chat::render(
            frame,
            chunks.chat,
            state,
            render_cache,
            plain_lines,
            tool_rows,
        );

        // Status bar.
        panels::status_bar::render(frame, chunks.status_bar, state);

        // Input area.
        panels::input::render(
            frame,
            chunks.input,
            state.focus,
            state.is_streaming,
            state.is_cancelling,
            state.follow_up_queue.len(),
            textarea,
            &state.theme,
        );
    }

    /// Convert terminal (x, y) to content-space (row, col) within the chat area.
    ///
    /// The returned `col` is a **character index**, not a display column,
    /// so that it aligns with `TextSelection` and `extract_text` which
    /// both operate on character indices.
    fn terminal_to_content(
        &self,
        x: u16,
        y: u16,
        chat_area: ratatui::layout::Rect,
    ) -> (usize, usize) {
        // Display column within the content area (after border).
        let display_col = (x.saturating_sub(chat_area.x).saturating_sub(1)) as usize;
        let content_y = (y.saturating_sub(chat_area.y).saturating_sub(1)) as usize;

        let inner_height = chat_area.height.saturating_sub(2) as usize;
        let total_lines = self.chat_plain_lines.len();
        let scroll_to =
            panels::chat::compute_scroll_to(total_lines, inner_height, self.state.scroll_offset);

        let row = scroll_to + content_y;

        // Convert display column -> character index using unicode widths.
        let char_idx = if let Some(line) = self.chat_plain_lines.get(row) {
            display_col_to_char_idx(line, display_col)
        } else {
            display_col
        };

        (row, char_idx)
    }

    /// Restore the terminal to its original state.
    fn restore_terminal(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        )?;
        self.terminal.show_cursor()?;
        Ok(())
    }

    /// Return the active session ID at exit time.
    pub fn exit_session_id(&self) -> Option<String> {
        self.state.current_session_id.clone()
    }

    /// Return cumulative input tokens at exit time.
    pub fn exit_input_tokens(&self) -> u64 {
        self.state.cumulative_input_tokens
    }

    /// Return cumulative output tokens at exit time.
    pub fn exit_output_tokens(&self) -> u64 {
        self.state.cumulative_output_tokens
    }

    /// Load session list from storage into state.
    async fn load_sessions(&mut self) {
        match self.workspace_sessions().await {
            Ok(nodes) => {
                self.state.sessions = nodes
                    .into_iter()
                    .map(|n| SessionListItem {
                        id: n.id.to_string(),
                        title: n.manual_title.or(n.title).unwrap_or_default(),
                        updated_at: n.updated_at,
                        message_count: n.message_count,
                    })
                    .collect();
            }
            Err(e) => {
                warn!(error = %e, "failed to load session list");
            }
        }
    }

    async fn workspace_sessions(&self) -> anyhow::Result<Vec<y_core::session::SessionNode>> {
        let workspace = std::env::current_dir()?;
        y_service::SessionService::list_resumable_sessions(
            &self.services.session_manager,
            &workspace,
            None,
        )
        .await
    }

    /// Switch the active session and load its transcript.
    ///
    /// On load failure the previous session ID is restored, so the UI never
    /// points at a session whose transcript was never loaded.
    async fn switch_active_session(
        &mut self,
        session_id: &y_core::types::SessionId,
    ) -> Result<(), String> {
        let previous = self.state.current_session_id.clone();
        self.state.current_session_id = Some(session_id.to_string());
        // The queue projection belongs to the previously active session.
        self.state.follow_up_queue.clear();
        match self.load_session_transcript(session_id).await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.state.current_session_id = previous;
                Err(error)
            }
        }
    }

    /// Load a session's transcript into the chat panel.
    async fn load_session_transcript(
        &mut self,
        session_id: &y_core::types::SessionId,
    ) -> Result<(), String> {
        let messages = match self
            .services
            .session_manager
            .read_display_transcript(session_id)
            .await
        {
            Ok(messages) => messages,
            Err(e) => {
                warn!(error = %e, "failed to load transcript");
                return Err(format!("Failed to load transcript: {e}"));
            }
        };

        // Reset cumulative status bar counters before re-accumulating.
        self.state.selected_tool = None;
        self.state.clear_backtrack_selection();
        self.state.cumulative_input_tokens = 0;
        self.state.cumulative_output_tokens = 0;
        self.state.last_cost = None;
        self.state.status_model = String::new();
        self.state.status_tokens = String::new();
        self.state.last_input_tokens = 0;

        self.state.messages = messages
            .into_iter()
            .map(|m| {
                let tool_calls = extract_tool_calls_from_metadata(&m.metadata);
                let segments = build_segments_from_content(&m.content, &tool_calls);

                // Accumulate status bar data from assistant metadata.
                if m.role == y_core::types::Role::Assistant {
                    restore_status_from_metadata(&m.metadata, &mut self.state);
                }

                state::ChatMessage {
                    role: match m.role {
                        y_core::types::Role::User => state::MessageRole::User,
                        y_core::types::Role::Assistant => state::MessageRole::Assistant,
                        y_core::types::Role::System => state::MessageRole::System,
                        y_core::types::Role::Tool => state::MessageRole::Tool,
                    },
                    content: m.content,
                    timestamp: m.timestamp,
                    is_streaming: false,
                    is_cancelled: false,
                    reasoning_content: String::new(),
                    reasoning_complete: false,
                    tool_calls,
                    segments,
                }
            })
            .collect();
        self.state.scroll_offset = 0;

        // Reset user message counter from transcript.
        self.state.user_message_count = u32::try_from(
            self.state
                .messages
                .iter()
                .filter(|m| matches!(m.role, state::MessageRole::User))
                .count(),
        )
        .unwrap_or(0);
        self.load_session_prompt_status(session_id).await;
        Ok(())
    }

    async fn load_session_prompt_status(&mut self, session_id: &y_core::types::SessionId) {
        let config = match y_service::PromptTemplateService::get_session_config(
            &self.services.session_manager,
            session_id,
        )
        .await
        {
            Ok(config) => config,
            Err(error) => {
                warn!(%error, "failed to load session prompt config");
                self.state.prompt_template_status = PromptTemplateStatus::Default;
                return;
            }
        };
        let templates = self
            .prompt_templates()
            .map(<[_]>::to_vec)
            .unwrap_or_default();
        self.state.prompt_template_status = resolve_prompt_template_status(&config, &templates);
    }
}

fn load_prompt_templates() -> Result<Vec<y_service::UserPromptTemplate>, String> {
    let config_dir = crate::config::dirs_user_config()
        .ok_or_else(|| "User config directory is unavailable.".to_string())?;
    y_service::load_user_prompt_templates(&config_dir)
        .map_err(|error| format!("Failed to load prompt templates: {error}"))
}

/// Find a session by ID prefix or case-insensitive title substring.
fn find_session_by_target<'a>(
    nodes: &'a [y_core::session::SessionNode],
    target: &str,
) -> Option<&'a y_core::session::SessionNode> {
    let target_lower = target.to_lowercase();
    nodes.iter().find(|node| {
        node.id.to_string().starts_with(target)
            || node
                .title
                .as_ref()
                .is_some_and(|title| title.to_lowercase().contains(&target_lower))
    })
}

/// Whether the input textarea holds no user text (all lines empty).
fn textarea_is_empty(textarea: &TextArea<'_>) -> bool {
    textarea.lines().iter().all(String::is_empty)
}

/// Cycle the selected tool card's detail level, auto-selecting the most
/// recent card when nothing is selected yet.
///
/// Returns `false` only when the transcript contains no tool cards at all.
fn toggle_tool_display(state: &mut AppState) -> bool {
    if state.selected_tool.is_none() {
        // From no selection, `select_previous_tool` wraps to the last card —
        // the one the user is most likely watching.
        state.select_previous_tool();
    }
    state.cycle_selected_tool_display().is_some()
}

/// Find the tool card covering the absolute chat-content `row`, if any.
fn tool_at_row(
    tool_rows: &[(std::ops::Range<usize>, ToolSelection)],
    row: usize,
) -> Option<ToolSelection> {
    tool_rows
        .iter()
        .find(|(range, _)| range.contains(&row))
        .map(|(_, selection)| *selection)
}

/// Recover UI state after the chat-event channel closed.
///
/// Terminal events (response/error/cancelled) clear the streaming flags
/// before the channel closes, so a close that still finds a streaming
/// assistant message means the service task died mid-turn: the partial
/// response is marked cancelled and the operator is warned. The service
/// destroys the follow-up queue when the run dies, so the projection is
/// cleared as well.
fn handle_chat_channel_closed(state: &mut AppState) {
    state.is_streaming = false;
    state.is_cancelling = false;
    state.follow_up_queue.clear();

    let interrupted = match state.messages.last_mut() {
        Some(last) if last.role == MessageRole::Assistant && last.is_streaming => {
            last.is_streaming = false;
            last.is_cancelled = true;
            true
        }
        _ => false,
    };
    if interrupted {
        state.push_toast(
            "Turn interrupted: connection to the service was lost.".into(),
            ToastLevel::Warning,
        );
    }
}

/// Selection highlight style for the input composer, matching the chat
/// panel's text-selection highlight (`apply_selection_highlight`).
fn input_selection_style() -> Style {
    Style::new()
        .fg(Color::Black)
        .bg(Color::White)
        .add_modifier(Modifier::BOLD)
}

/// Mirror of tui-textarea's internal `next_scroll_top` (widget.rs): the
/// viewport top follows the cursor so it stays inside `[top, top + len)`.
/// Replicated because the crate exposes no scroll-offset getter.
fn next_scroll_top(prev_top: u16, cursor: u16, len: u16) -> u16 {
    if cursor < prev_top {
        cursor
    } else if prev_top.saturating_add(len) <= cursor {
        cursor.saturating_add(1).saturating_sub(len)
    } else {
        prev_top
    }
}

/// Map a terminal position inside the input panel to a composer buffer
/// position `(row, col)`, accounting for the border (+1) and the tracked
/// vertical scroll offset. Horizontal scrolling is ignored: composer lines
/// are assumed to fit the panel width, so a click past a line's end simply
/// clamps to the line end via `CursorMove::Jump`.
fn input_buffer_position(
    input: ratatui::layout::Rect,
    input_vscroll: u16,
    x: u16,
    y: u16,
) -> (u16, u16) {
    let row = y
        .saturating_sub(input.y)
        .saturating_sub(1)
        .saturating_add(input_vscroll);
    let col = x.saturating_sub(input.x).saturating_sub(1);
    (row, col)
}

/// Collapse bracketed-paste text for a single-line filter input.
///
/// Picker and palette filters are single-line queries, so line breaks are
/// dropped rather than embedded in the query.
fn single_line_paste_text(text: &str) -> String {
    text.chars()
        .filter(|ch| !matches!(ch, '\n' | '\r'))
        .collect()
}

fn resolve_prompt_template_status(
    config: &y_service::SessionPromptConfig,
    templates: &[y_service::UserPromptTemplate],
) -> PromptTemplateStatus {
    if let Some(template_id) = config.template_id.as_deref() {
        let name = templates
            .iter()
            .find(|template| template.id == template_id)
            .map_or_else(|| template_id.to_string(), |template| template.name.clone());
        return PromptTemplateStatus::Template {
            id: template_id.to_string(),
            name,
        };
    }
    if y_service::session_prompt_config_has_content(config) {
        PromptTemplateStatus::Custom
    } else {
        PromptTemplateStatus::Default
    }
}

/// Convert a display-column offset to a character index within a string.
///
/// Walks through `text` accumulating each character's display width
/// (via `unicode_width`) until the accumulated width reaches or exceeds
/// `display_col`. Returns the 0-based character index at that point.
fn display_col_to_char_idx(text: &str, display_col: usize) -> usize {
    let mut col = 0usize;
    for (i, ch) in text.chars().enumerate() {
        if col >= display_col {
            return i;
        }
        col += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    // Past end of line -- clamp to character count.
    text.chars().count()
}

/// Check if a point (x, y) is inside a `Rect`.
fn contains(rect: ratatui::layout::Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

/// Whether a tick must trigger a redraw.
///
/// Idle ticks skip rendering entirely; a redraw is only needed while
/// something time-dependent is on screen: streaming animations (spinner),
/// visible toasts (countdown/expiry), or a toast state change this tick.
fn tick_marks_dirty(is_streaming: bool, toasts_visible: bool, toasts_changed: bool) -> bool {
    is_streaming || toasts_visible || toasts_changed
}

/// Interval between background-task list polls, in 100 ms ticks (~1.5 s).
const BG_POLL_INTERVAL_TICKS: u64 = 15;

/// Whether a background-task list poll should run on this tick.
///
/// Polling is throttled to every [`BG_POLL_INTERVAL_TICKS`] ticks and only
/// runs when there is something to observe: an active stream, known
/// background activity, or the open `/tasks` overlay. When idle with zero
/// counts, polls are skipped entirely.
fn bg_poll_due(
    tick_counter: u64,
    is_streaming: bool,
    bg_task_count: usize,
    active_subagent_count: usize,
    tasks_overlay_open: bool,
) -> bool {
    let relevant =
        is_streaming || bg_task_count > 0 || active_subagent_count > 0 || tasks_overlay_open;
    relevant && tick_counter.is_multiple_of(BG_POLL_INTERVAL_TICKS)
}

fn backtrack_branch_title(prompt: &str) -> String {
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return "Backtrack branch".to_string();
    }

    let mut chars = normalized.chars();
    let summary: String = chars.by_ref().take(48).collect();
    if chars.next().is_some() {
        format!(
            "Backtrack: {}...",
            summary.chars().take(45).collect::<String>()
        )
    } else {
        format!("Backtrack: {summary}")
    }
}

/// Ensure terminal is restored even if `TuiApp` is dropped without calling
/// `restore_terminal` (e.g., on panic).
impl Drop for TuiApp {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

// ---------------------------------------------------------------------------
// Transcript restoration helpers
// ---------------------------------------------------------------------------

/// Extract tool call info from an assistant message's `metadata.tool_results`.
fn extract_tool_calls_from_metadata(metadata: &serde_json::Value) -> Vec<state::ToolCallInfo> {
    let Some(results) = metadata.get("tool_results").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    results
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let name = entry.get("name")?.as_str()?.to_string();
            let tool_call_id = entry
                .get("tool_call_id")
                .and_then(serde_json::Value::as_str)
                .map_or_else(|| format!("legacy-tool-{index}"), str::to_string);
            let success = entry
                .get("success")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let duration_ms = entry
                .get("duration_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let input_preview = metadata_value_as_text(entry.get("arguments"));
            let result_preview = metadata_value_as_text(entry.get("result_preview"));
            let url_meta = entry.get("url_meta").map(metadata_value_to_text);
            Some(state::ToolCallInfo {
                tool_call_id,
                name,
                status: if success {
                    state::ToolCallStatus::Succeeded
                } else {
                    state::ToolCallStatus::Failed
                },
                duration_ms: Some(duration_ms),
                input_preview,
                result_preview,
                agent_name: entry
                    .get("agent_name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                url_meta,
                metadata: entry.get("metadata").cloned(),
                display_mode: state::ToolCallDisplayMode::Preview,
            })
        })
        .collect()
}

fn metadata_value_as_text(value: Option<&serde_json::Value>) -> String {
    value.map_or_else(String::new, metadata_value_to_text)
}

fn metadata_value_to_text(value: &serde_json::Value) -> String {
    value.as_str().map_or_else(
        || serde_json::to_string(value).unwrap_or_default(),
        str::to_string,
    )
}

/// Build event-ordered segments from content text and tool calls.
///
/// For historical messages we don't have the original interleaving, so we
/// emit the full text first, then append tool call segments.
fn build_segments_from_content(
    content: &str,
    tool_calls: &[state::ToolCallInfo],
) -> Vec<state::StreamSegment> {
    if tool_calls.is_empty() {
        return Vec::new();
    }
    let mut segments = Vec::with_capacity(1 + tool_calls.len());
    if !content.is_empty() {
        segments.push(state::StreamSegment::Text(content.to_string()));
    }
    for tool_index in 0..tool_calls.len() {
        segments.push(state::StreamSegment::ToolCall(tool_index));
    }
    segments
}

/// Restore status bar fields from a single assistant message's metadata.
///
/// Called for each assistant message during transcript loading. Accumulates
/// cumulative token counts and overwrites "last" fields so the final call
/// (the most recent assistant message) leaves the correct current values.
fn restore_status_from_metadata(metadata: &serde_json::Value, state: &mut state::AppState) {
    let input_tokens = metadata
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let output_tokens = metadata
        .get("output_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    state.cumulative_input_tokens += input_tokens;
    state.cumulative_output_tokens += output_tokens;

    if let Some(model) = metadata.get("model").and_then(|v| v.as_str()) {
        state.status_model = model.to_string();
    }

    if let Some(requested_mode) = metadata
        .get("orchestration")
        .and_then(|value| value.get("requested_mode"))
        .and_then(serde_json::Value::as_str)
        .and_then(state::TurnMode::parse)
    {
        state.turn_mode = requested_mode;
    }

    state.status_tokens = format!(
        "{}\u{2191} {}\u{2193}",
        state.cumulative_input_tokens, state.cumulative_output_tokens
    );

    if let Some(ctx) = metadata
        .get("context_tokens_used")
        .and_then(serde_json::Value::as_u64)
    {
        state.last_input_tokens = ctx;
    }

    if let Some(window) = metadata
        .get("context_window")
        .and_then(serde_json::Value::as_u64)
    {
        if window > 0 {
            state.context_window = window as usize;
        }
    }

    if let Some(cost) = metadata.get("cost_usd").and_then(serde_json::Value::as_f64) {
        if cost > 0.0 {
            state.last_cost = Some(state.last_cost.unwrap_or(0.0) + cost);
        }
    }
}

#[cfg(test)]
mod transcript_tests {
    use super::*;

    // T-REDRAW-01: idle ticks must not request a redraw; time-dependent UI
    // (streaming animation, toast countdowns, toast changes) must.
    #[test]
    fn test_tick_marks_dirty_truth_table() {
        assert!(!tick_marks_dirty(false, false, false), "idle tick");
        assert!(tick_marks_dirty(true, false, false), "streaming animation");
        assert!(tick_marks_dirty(false, true, false), "toast countdown");
        assert!(
            tick_marks_dirty(false, false, true),
            "toast pushed or expired"
        );
        assert!(tick_marks_dirty(true, true, true), "any activity");
    }

    // Background polls are throttled to the poll interval and only run while
    // something observable exists (stream, non-zero counts, open overlay).
    #[test]
    fn test_bg_poll_due_throttles_and_gates() {
        // Idle with zero counts and the overlay closed: never due.
        assert!(!bg_poll_due(0, false, 0, 0, false));
        assert!(!bg_poll_due(15, false, 0, 0, false));
        // Relevant but off-interval ticks are skipped.
        assert!(!bg_poll_due(1, true, 0, 0, false));
        assert!(!bg_poll_due(14, false, 2, 0, false));
        assert!(!bg_poll_due(29, false, 0, 0, true));
        // Due on the interval when anything is observable.
        assert!(bg_poll_due(15, true, 0, 0, false), "streaming");
        assert!(bg_poll_due(30, false, 1, 0, false), "known bg tasks");
        assert!(bg_poll_due(45, false, 0, 3, false), "known subagents");
        assert!(bg_poll_due(60, false, 0, 0, true), "overlay open");
    }

    #[test]
    fn test_extract_tool_calls_from_metadata_preserves_rich_fields() {
        let metadata = serde_json::json!({
            "tool_results": [{
                "tool_call_id": "call-edit-1",
                "name": "FileEdit",
                "arguments": {"path": "src/main.rs", "old": "a", "new": "b"},
                "success": true,
                "duration_ms": 17,
                "result_preview": "updated src/main.rs",
                "url_meta": {"url": "https://example.com"},
                "metadata": {"changed_lines": 1}
            }]
        });

        let calls = extract_tool_calls_from_metadata(&metadata);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_call_id, "call-edit-1");
        assert_eq!(calls[0].status, state::ToolCallStatus::Succeeded);
        assert_eq!(calls[0].duration_ms, Some(17));
        assert!(calls[0].input_preview.contains("src/main.rs"));
        assert_eq!(calls[0].result_preview, "updated src/main.rs");
        assert!(calls[0]
            .url_meta
            .as_deref()
            .is_some_and(|value| value.contains("url")));
        assert_eq!(
            calls[0].metadata,
            Some(serde_json::json!({"changed_lines": 1}))
        );
    }

    #[test]
    fn test_extract_legacy_tool_metadata_assigns_presentation_fallback_id() {
        let metadata = serde_json::json!({
            "tool_results": [{
                "name": "FileRead",
                "success": true,
                "duration_ms": 1,
                "result_preview": "contents"
            }]
        });

        let calls = extract_tool_calls_from_metadata(&metadata);

        assert_eq!(calls.len(), 1);
        assert!(calls[0].tool_call_id.starts_with("legacy-tool-"));
    }

    #[test]
    fn test_backtrack_branch_title_is_compact() {
        assert_eq!(
            backtrack_branch_title("  fix\nthis bug "),
            "Backtrack: fix this bug"
        );
        assert!(backtrack_branch_title(&"x".repeat(80)).chars().count() <= 62);
        assert_eq!(backtrack_branch_title("  "), "Backtrack branch");
    }

    // Regression: ':' typed into a non-empty draft (e.g. "12:30", URLs) must
    // stay literal text; command mode is only entered from an empty buffer.
    #[test]
    fn test_textarea_is_empty_gates_command_mode() {
        assert!(textarea_is_empty(&TextArea::default()));
        assert!(textarea_is_empty(&TextArea::new(vec![
            String::new(),
            String::new()
        ])));
        assert!(!textarea_is_empty(&TextArea::new(vec!["12".to_string()])));
        assert!(!textarea_is_empty(&TextArea::new(vec![
            String::new(),
            "https://example.com".to_string()
        ])));
    }

    // Bracketed paste: picker/palette filters are single-line, so pasted
    // line breaks are dropped; other characters pass through untouched.
    #[test]
    fn test_single_line_paste_text_strips_line_breaks() {
        assert_eq!(single_line_paste_text("a\nb\r\nc"), "abc");
        assert_eq!(single_line_paste_text("plain text"), "plain text");
        assert_eq!(single_line_paste_text(""), "");
    }

    fn session_node(id: &str, title: Option<&str>) -> y_core::session::SessionNode {
        let sid = y_core::types::SessionId::from_string(id);
        y_core::session::SessionNode {
            id: sid.clone(),
            parent_id: None,
            root_id: sid.clone(),
            depth: 0,
            path: vec![sid],
            session_type: y_core::session::SessionType::Main,
            state: y_core::session::SessionState::Active,
            agent_id: None,
            title: title.map(str::to_string),
            manual_title: None,
            channel: None,
            label: None,
            workspace_path: None,
            token_count: 0,
            message_count: 0,
            last_compaction: None,
            compaction_count: 0,
            branch_summary: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_find_session_by_target_matches_id_prefix_and_title() {
        let nodes = vec![
            session_node(
                "abcd1234-0000-0000-0000-000000000000",
                Some("Fix Login Bug"),
            ),
            session_node("efgh5678-0000-0000-0000-000000000000", None),
        ];

        // ID prefix match.
        let by_id = find_session_by_target(&nodes, "efgh5678");
        assert_eq!(
            by_id.map(|n| n.id.to_string()),
            Some("efgh5678-0000-0000-0000-000000000000".to_string())
        );

        // Title substring match, case-insensitive.
        let by_title = find_session_by_target(&nodes, "login bug");
        assert_eq!(
            by_title.map(|n| n.id.to_string()),
            Some("abcd1234-0000-0000-0000-000000000000".to_string())
        );

        // No match.
        assert!(find_session_by_target(&nodes, "nonexistent").is_none());
    }

    #[test]
    fn test_resolve_prompt_template_status_handles_default_template_and_custom() {
        let templates = vec![y_service::UserPromptTemplate {
            id: "review".into(),
            name: "Reviewer".into(),
            description: None,
            system_prompt: "Review carefully.".into(),
            prompt_section_ids: Vec::new(),
        }];
        let template_config = y_service::SessionPromptConfig {
            system_prompt: Some("Review carefully.".into()),
            prompt_section_ids: Vec::new(),
            template_id: Some("review".into()),
        };
        assert_eq!(
            resolve_prompt_template_status(&template_config, &templates),
            PromptTemplateStatus::Template {
                id: "review".into(),
                name: "Reviewer".into(),
            }
        );

        let custom_config = y_service::SessionPromptConfig {
            system_prompt: Some("Custom".into()),
            prompt_section_ids: Vec::new(),
            template_id: None,
        };
        assert_eq!(
            resolve_prompt_template_status(&custom_config, &templates),
            PromptTemplateStatus::Custom
        );
        assert_eq!(
            resolve_prompt_template_status(&y_service::SessionPromptConfig::default(), &templates),
            PromptTemplateStatus::Default
        );
    }
}

// ---------------------------------------------------------------------------
// Interaction tests: tool toggles, mouse hit-mapping, disconnect recovery
// ---------------------------------------------------------------------------

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use chrono::Utc;

    fn tool_call(name: &str) -> state::ToolCallInfo {
        state::ToolCallInfo {
            tool_call_id: format!("call-{name}"),
            name: name.to_string(),
            status: state::ToolCallStatus::Succeeded,
            duration_ms: Some(1),
            input_preview: String::new(),
            result_preview: String::new(),
            agent_name: String::new(),
            url_meta: None,
            metadata: None,
            display_mode: state::ToolCallDisplayMode::Preview,
        }
    }

    fn assistant_message(is_streaming: bool, tool_names: &[&str]) -> ChatMessage {
        ChatMessage {
            role: MessageRole::Assistant,
            content: "answer".to_string(),
            timestamp: Utc::now(),
            is_streaming,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: true,
            tool_calls: tool_names.iter().map(|name| tool_call(name)).collect(),
            segments: Vec::new(),
        }
    }

    // Ctrl+O with no selection must target the most recent tool card, not
    // the first one in the transcript.
    #[test]
    fn test_toggle_tool_display_autoselects_latest_card() {
        let mut state = AppState::new();
        state.messages.push(assistant_message(false, &["Read"]));
        state
            .messages
            .push(assistant_message(false, &["Edit", "Bash"]));

        assert!(toggle_tool_display(&mut state));
        assert_eq!(
            state.selected_tool,
            Some(ToolSelection {
                message_index: 1,
                tool_index: 1
            })
        );
        // Preview -> Expanded on the first toggle.
        assert_eq!(
            state.messages[1].tool_calls[1].display_mode,
            state::ToolCallDisplayMode::Expanded
        );

        // A second toggle cycles the same card (Expanded -> Collapsed).
        assert!(toggle_tool_display(&mut state));
        assert_eq!(
            state.messages[1].tool_calls[1].display_mode,
            state::ToolCallDisplayMode::Collapsed
        );
    }

    #[test]
    fn test_toggle_tool_display_without_cards_returns_false() {
        let mut state = AppState::new();
        state.messages.push(assistant_message(false, &[]));

        assert!(!toggle_tool_display(&mut state));
        assert!(state.selected_tool.is_none());
    }

    #[test]
    fn test_tool_at_row_hit_lookup() {
        let rows = vec![
            (
                3..6,
                ToolSelection {
                    message_index: 0,
                    tool_index: 0,
                },
            ),
            (
                10..12,
                ToolSelection {
                    message_index: 1,
                    tool_index: 0,
                },
            ),
        ];

        let first = Some(ToolSelection {
            message_index: 0,
            tool_index: 0,
        });
        let second = Some(ToolSelection {
            message_index: 1,
            tool_index: 0,
        });
        assert_eq!(tool_at_row(&rows, 3), first, "range start is inclusive");
        assert_eq!(tool_at_row(&rows, 5), first);
        assert_eq!(tool_at_row(&rows, 11), second);
        // Range end is exclusive; gaps and beyond-content rows miss.
        assert_eq!(tool_at_row(&rows, 6), None);
        assert_eq!(tool_at_row(&rows, 8), None);
        assert_eq!(tool_at_row(&rows, 12), None);
        assert_eq!(tool_at_row(&rows, 0), None);
    }

    // Disconnect mid-turn: the partial assistant response is marked
    // cancelled, the queue projection is dropped, and a warning toast shows.
    #[test]
    fn test_channel_close_marks_streaming_turn_interrupted() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.is_cancelling = true;
        state
            .follow_up_queue
            .push(y_service::FollowUpMessage::new("queued".to_string()));
        state.messages.push(assistant_message(true, &[]));

        handle_chat_channel_closed(&mut state);

        assert!(!state.is_streaming);
        assert!(!state.is_cancelling);
        assert!(state.follow_up_queue.is_empty());
        let last = &state.messages[0];
        assert!(!last.is_streaming);
        assert!(last.is_cancelled);
        assert_eq!(state.toasts.len(), 1);
        let toast = &state.toasts[0];
        assert_eq!(toast.level, ToastLevel::Warning);
        assert!(
            toast.message.contains("Turn interrupted"),
            "unexpected toast: {}",
            toast.message
        );
    }

    // A channel close after a cleanly completed turn is the normal path: no
    // message mutation, no toast (but the queue projection still clears).
    #[test]
    fn test_channel_close_after_clean_completion_stays_quiet() {
        let mut state = AppState::new();
        state
            .follow_up_queue
            .push(y_service::FollowUpMessage::new("queued".to_string()));
        state.messages.push(assistant_message(false, &[]));

        handle_chat_channel_closed(&mut state);

        assert!(state.follow_up_queue.is_empty());
        assert!(state.toasts.is_empty());
        assert!(!state.messages[0].is_cancelled);
    }

    #[test]
    fn test_channel_close_with_empty_transcript_does_not_panic() {
        let mut state = AppState::new();
        handle_chat_channel_closed(&mut state);
        assert!(state.toasts.is_empty());
    }

    #[test]
    fn test_next_scroll_top_follows_cursor() {
        // Cursor above the viewport pulls the top up.
        assert_eq!(next_scroll_top(4, 2, 6), 2);
        // Cursor below the viewport pushes the top down (cursor lands on the
        // last visible row).
        assert_eq!(next_scroll_top(0, 9, 6), 4);
        // Cursor inside the viewport leaves the top unchanged (rows 2..=7
        // visible for top=2, len=6).
        assert_eq!(next_scroll_top(2, 2, 6), 2);
        assert_eq!(next_scroll_top(2, 5, 6), 2);
        assert_eq!(next_scroll_top(2, 7, 6), 2);
        assert_eq!(next_scroll_top(2, 8, 6), 3);
    }

    #[test]
    fn test_input_buffer_position_maps_clicks() {
        let input = ratatui::layout::Rect::new(0, 20, 80, 8); // inner rows 21..=27
                                                              // Top-left inner cell with no scroll.
        assert_eq!(input_buffer_position(input, 0, 1, 21), (0, 0));
        // Row/column offsets from the border.
        assert_eq!(input_buffer_position(input, 0, 6, 23), (2, 5));
        // Scrolled composer: the first visible row maps to the scroll top.
        assert_eq!(input_buffer_position(input, 4, 1, 21), (4, 0));
        assert_eq!(input_buffer_position(input, 4, 6, 23), (6, 5));
        // Clicks on the border saturate to the first inner cell.
        assert_eq!(input_buffer_position(input, 0, 0, 20), (0, 0));
    }
}
