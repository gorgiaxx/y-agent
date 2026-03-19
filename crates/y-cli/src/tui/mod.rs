//! TUI application shell — entry point, terminal setup, main event loop.
//!
//! `TuiApp` manages the ratatui terminal lifecycle (raw mode, alternate screen)
//! and drives the render–event–update loop. It delegates rendering to panel
//! modules and key handling to the key dispatcher (both in Phase T3+).
//!
//! NOTE: `dead_code` is allowed at module level because this is a scaffold —
//! many state model types and methods will be consumed in later phases.
#![allow(dead_code)]

pub mod chat_flow;
pub mod commands;
pub mod events;
pub mod keys;
pub mod layout;
pub mod overlays;
pub mod panels;
pub mod selection;
pub mod state;
pub mod tracing_bridge;

use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
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
use tui_textarea::TextArea;

use crate::wire::AppServices;
use chat_flow::ChatEvent;
use commands::handlers::{self, CommandResult};
use events::{AppEvent, EventLoop};
use keys::KeyAction;
use layout::LayoutChunks;
use overlays::command_palette::CommandPaletteState;
use state::{AppState, InteractionMode, PanelFocus, SessionListItem, Toast, ToastLevel};

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
    /// Application services (LLM, session, etc.).
    services: Arc<AppServices>,
    /// Receiver for LLM response events.
    llm_rx: Option<tokio::sync::mpsc::Receiver<ChatEvent>>,
    /// Receiver for toast messages from the tracing bridge.
    toast_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Toast>>,
    /// Last computed layout chunks for mouse hit-testing.
    last_chunks: Option<LayoutChunks>,
    /// Cached plain-text lines from last chat render (for selection extraction).
    chat_plain_lines: Vec<String>,
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
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        let state = AppState::new();
        let events = EventLoop::new(Duration::from_millis(250));
        let textarea = TextArea::default();
        let palette = CommandPaletteState::new();

        Ok(Self {
            terminal,
            state,
            events,
            textarea,
            palette,
            services,
            llm_rx: None,
            toast_rx,
            last_chunks: None,
            chat_plain_lines: Vec::new(),
        })
    }

    /// Run the TUI main loop.
    ///
    /// Returns when the user presses `Ctrl+Q`, `Ctrl+D`, or `Ctrl+C`.
    /// Terminal cleanup (raw mode off, leave alternate screen) is guaranteed
    /// via the `restore_terminal` call in all exit paths.
    pub async fn run(&mut self) -> Result<()> {
        // Load session list and create/resume a session at startup.
        self.load_sessions().await;
        self.ensure_current_session();

        // Initialize context_window from the default provider's metadata.
        if let Some(meta) = self.services.provider_pool().await.list_metadata().first() {
            self.state.context_window = meta.context_window;
        }

        loop {
            // 1. Render.
            self.draw()?;

            // 2. Wait for next event.
            let Some(event) = self.events.next().await else {
                break; // Event loop dropped.
            };

            // 3. Handle event.
            match event {
                AppEvent::Key(key) => {
                    let action = keys::dispatch(key, &self.state);
                    match action {
                        KeyAction::Quit => break,
                        KeyAction::Submit => {
                            if self.state.mode == InteractionMode::Command {
                                // Execute the command.
                                let cmd_input =
                                    if let Some(selected) = self.palette.selected_command() {
                                        selected.to_string()
                                    } else {
                                        self.palette.input.clone()
                                    };
                                self.execute_command(&cmd_input);
                                self.palette = CommandPaletteState::new();
                                self.state.set_mode(InteractionMode::Normal);
                                self.state.set_focus(PanelFocus::Input);
                            } else {
                                // Normal submit: check for slash commands first.
                                let input: String = self.textarea.lines().join("\n");
                                let trimmed = input.trim();
                                if trimmed.starts_with('/') && !trimmed.is_empty() {
                                    // Route through command system (strip the leading '/').
                                    let cmd_input = &trimmed[1..];
                                    self.state.push_history(trimmed);
                                    self.execute_command(cmd_input);
                                } else if !trimmed.is_empty() {
                                    // Push to history and send message to LLM.
                                    self.state.push_history(trimmed);
                                    self.llm_rx = chat_flow::submit_message(
                                        &input,
                                        &mut self.state,
                                        &self.services,
                                    );
                                }
                                self.textarea = TextArea::default();
                            }
                        }
                        KeyAction::InputPassthrough => {
                            if self.state.mode == InteractionMode::Command {
                                // Forward to palette input.
                                if let crossterm::event::KeyCode::Char(ch) = key.code {
                                    self.palette.push_char(ch);
                                } else if key.code == crossterm::event::KeyCode::Backspace {
                                    self.palette.pop_char();
                                }
                            } else {
                                // Check if `/` typed on empty input → enter command mode.
                                if key.code == crossterm::event::KeyCode::Char('/')
                                    && self
                                        .textarea
                                        .lines()
                                        .iter()
                                        .all(std::string::String::is_empty)
                                {
                                    self.state.set_mode(InteractionMode::Command);
                                    self.palette = CommandPaletteState::new();
                                } else {
                                    self.textarea.input(key);
                                }
                            }
                        }
                        KeyAction::ToggleSidebar => {
                            self.state.toggle_sidebar();
                        }
                        KeyAction::ToggleSidebarView => {
                            self.state.toggle_sidebar_view();
                        }
                        KeyAction::CycleFocus => {
                            self.state.cycle_focus_forward();
                        }
                        KeyAction::ScrollUp => {
                            if self.state.mode == InteractionMode::Command {
                                self.palette.select_prev();
                            } else if self.state.focus == PanelFocus::Sidebar {
                                self.state.select_session_prev();
                            } else {
                                self.state.scroll_offset =
                                    self.state.scroll_offset.saturating_add(3);
                            }
                        }
                        KeyAction::ScrollDown => {
                            if self.state.mode == InteractionMode::Command {
                                self.palette.select_next();
                            } else if self.state.focus == PanelFocus::Sidebar {
                                self.state.select_session_next();
                            } else {
                                self.state.scroll_offset =
                                    self.state.scroll_offset.saturating_sub(3);
                            }
                        }
                        KeyAction::EnterCommandMode => {
                            self.state.set_mode(InteractionMode::Command);
                            self.palette = CommandPaletteState::new();
                        }
                        KeyAction::ReturnToNormal => {
                            self.state.set_mode(InteractionMode::Normal);
                            self.state.set_focus(PanelFocus::Input);
                            self.palette = CommandPaletteState::new();
                        }
                        KeyAction::HistoryPrev => {
                            if let Some(entry) = self.state.history_prev() {
                                self.textarea = TextArea::new(vec![entry.to_string()]);
                            }
                        }
                        KeyAction::HistoryNext => match self.state.history_next() {
                            Some(entry) => {
                                self.textarea = TextArea::new(vec![entry.to_string()]);
                            }
                            None => {
                                self.textarea = TextArea::default();
                            }
                        },
                        KeyAction::Consumed | KeyAction::Unhandled => {}
                        KeyAction::SelectSessionItem => {
                            self.switch_to_selected_session().await;
                        }
                    }
                }
                AppEvent::Mouse(mouse) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            // Hit-test click position against last layout chunks.
                            if let Some(ref chunks) = self.last_chunks {
                                let (x, y) = (mouse.column, mouse.row);
                                if contains(chunks.input, x, y) {
                                    self.state.set_focus(PanelFocus::Input);
                                    self.state.selection.reset();
                                } else if contains(chunks.chat, x, y) {
                                    self.state.set_focus(PanelFocus::Chat);
                                    // Start text selection in chat.
                                    let (row, col) = self.terminal_to_content(x, y, chunks.chat);
                                    self.state.selection.start(row, col);
                                } else if let Some(sb) = chunks.sidebar {
                                    if contains(sb, x, y) {
                                        self.state.set_focus(PanelFocus::Sidebar);
                                        self.state.selection.reset();
                                    }
                                }
                            }
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            // Update selection during drag.
                            if self.state.selection.active {
                                if let Some(ref chunks) = self.last_chunks {
                                    let (row, col) = self.terminal_to_content(
                                        mouse.column,
                                        mouse.row,
                                        chunks.chat,
                                    );
                                    self.state.selection.update(row, col);
                                }
                            }
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            // Finish selection and copy to clipboard.
                            if self.state.selection.active {
                                if let Some(ref chunks) = self.last_chunks {
                                    let (row, col) = self.terminal_to_content(
                                        mouse.column,
                                        mouse.row,
                                        chunks.chat,
                                    );
                                    self.state.selection.update(row, col);
                                }
                                self.state.selection.finish();

                                if !self.state.selection.is_empty() {
                                    let text = selection::extract_text(
                                        &self.chat_plain_lines,
                                        &self.state.selection,
                                    );
                                    if !text.is_empty() {
                                        #[cfg(feature = "tui")]
                                        if let Ok(mut clip) = arboard::Clipboard::new() {
                                            let _ = clip.set_text(&text);
                                        }
                                    }
                                }
                            }
                        }
                        MouseEventKind::Down(_) => {
                            // Non-left clicks: just reset selection.
                            self.state.selection.reset();
                        }
                        MouseEventKind::ScrollUp => {
                            let over_sidebar = self
                                .last_chunks
                                .as_ref()
                                .and_then(|c| c.sidebar)
                                .is_some_and(|sb| contains(sb, mouse.column, mouse.row));
                            if over_sidebar {
                                self.state.select_session_prev();
                            } else {
                                self.state.scroll_offset =
                                    self.state.scroll_offset.saturating_add(3);
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            let over_sidebar = self
                                .last_chunks
                                .as_ref()
                                .and_then(|c| c.sidebar)
                                .is_some_and(|sb| contains(sb, mouse.column, mouse.row));
                            if over_sidebar {
                                self.state.select_session_next();
                            } else {
                                self.state.scroll_offset =
                                    self.state.scroll_offset.saturating_sub(3);
                            }
                        }
                        _ => {}
                    }
                }
                AppEvent::Resize(_w, _h) => {
                    // Terminal automatically knows the new size on next draw.
                }
                AppEvent::Tick => {
                    // Drain toast channel from tracing bridge.
                    if let Some(ref mut rx) = self.toast_rx {
                        while let Ok(toast) = rx.try_recv() {
                            self.state.push_toast(toast.message, toast.level);
                        }
                    }

                    // Tick toast timers and expire old toasts.
                    self.state.tick_toasts();

                    // Poll LLM response channel.
                    if let Some(ref mut rx) = self.llm_rx {
                        // Drain all available events (e.g., Response + TitleUpdated).
                        let mut channel_closed = false;
                        loop {
                            match rx.try_recv() {
                                Ok(event) => {
                                    chat_flow::apply_chat_event(event, &mut self.state);
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
                            self.state.is_streaming = false;
                            self.llm_rx = None;
                        }
                    }
                }
            }
        }

        self.restore_terminal()?;
        Ok(())
    }

    /// Draw the current frame.
    fn draw(&mut self) -> Result<()> {
        // Pre-compute input height and layout outside the closure so we
        // can store them for mouse hit-testing.
        let input_lines = panels::input::input_height(&self.textarea);
        let term_size = self.terminal.size()?;
        let term_rect = ratatui::layout::Rect::new(0, 0, term_size.width, term_size.height);
        let chunks = layout::compute_layout(term_rect, self.state.sidebar_visible, input_lines);
        self.last_chunks = Some(chunks.clone());

        let state = &self.state;
        let textarea = &mut self.textarea;
        let palette = &self.palette;
        let chunks_ref = &chunks;
        let plain_lines_cell = std::cell::RefCell::new(Vec::<String>::new());

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
                            "Minimum: {}×{} — Current: {}×{}",
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

            let pl = Self::render_panels(frame, chunks, state, textarea);
            *plain_lines_cell.borrow_mut() = pl;

            // Render command palette overlay if in Command mode.
            if state.mode == InteractionMode::Command {
                overlays::command_palette::render(frame, area, palette);
            }

            // Render toast overlay (always, non-modal).
            overlays::toast::render(frame, area, &state.toasts);
        })?;

        // Cache plain text lines rendered by the chat panel.
        self.chat_plain_lines = plain_lines_cell.into_inner();

        Ok(())
    }

    /// Execute a command and apply its result to state.
    fn execute_command(&mut self, cmd_input: &str) {
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
                // Handled in the run loop.
            }
            CommandResult::Ok(None) => {
                // Handler already modified state directly.
            }
            CommandResult::NewSession => {
                // State has been reset by the handler (messages cleared,
                // current_session_id set to None, user_message_count reset).
                // Actual session creation is deferred to first message.
                self.state.sync_selected_session_index();
                self.state
                    .push_toast("New session started.".into(), ToastLevel::Info);
            }
        }
    }

    /// Render all panels into their layout chunks.
    /// Returns the plain-text lines from the chat panel for selection extraction.
    fn render_panels(
        frame: &mut ratatui::Frame,
        chunks: &LayoutChunks,
        state: &AppState,
        textarea: &TextArea<'_>,
    ) -> Vec<String> {
        // Sidebar (if visible).
        if let Some(sidebar_area) = chunks.sidebar {
            panels::sidebar::render(frame, sidebar_area, state);
        }

        // Chat panel — returns plain text lines for selection.
        let plain_lines = panels::chat::render(frame, chunks.chat, state);

        // Status bar.
        panels::status_bar::render(frame, chunks.status_bar, state);

        // Input area.
        panels::input::render(frame, chunks.input, state.focus, textarea);

        plain_lines
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

        // Compute scroll offset (same logic as chat.rs).
        let inner_height = chat_area.height.saturating_sub(2) as usize;
        let total_lines = self.chat_plain_lines.len();
        let scroll_to = if self.state.scroll_offset == 0 {
            total_lines.saturating_sub(inner_height)
        } else {
            total_lines
                .saturating_sub(inner_height)
                .saturating_sub(self.state.scroll_offset)
        };

        let row = scroll_to + content_y;

        // Convert display column → character index using unicode widths.
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
            LeaveAlternateScreen
        )?;
        self.terminal.show_cursor()?;
        Ok(())
    }

    /// Load session list from storage into state.
    async fn load_sessions(&mut self) {
        use y_core::session::SessionFilter;

        match self
            .services
            .session_manager
            .list_sessions(&SessionFilter::default())
            .await
        {
            Ok(mut nodes) => {
                // Sort by updated_at descending (most recent first).
                nodes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

                self.state.sessions = nodes
                    .into_iter()
                    .map(|n| SessionListItem {
                        id: n.id.to_string(),
                        title: n.title.unwrap_or_default(),
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

    /// Ensure there is a current session. On fresh startup, we do NOT resume
    /// the most recent session — instead, we leave `current_session_id = None`
    /// so the user always starts with a clean slate. The session will be created
    /// lazily when the first message is sent (see `chat_flow::submit_message`).
    fn ensure_current_session(&mut self) {
        // Nothing to do — lazy creation on first message.
    }

    /// Load a session's transcript into the chat panel.
    async fn load_session_transcript(&mut self, session_id: &y_core::types::SessionId) {
        match self
            .services
            .session_manager
            .read_transcript(session_id)
            .await
        {
            Ok(messages) => {
                self.state.messages = messages
                    .into_iter()
                    .map(|m| state::ChatMessage {
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
            }
            Err(e) => {
                warn!(error = %e, "failed to load transcript");
            }
        }
    }

    /// Switch to the currently selected session in the sidebar.
    async fn switch_to_selected_session(&mut self) {
        let Some(idx) = self.state.selected_session_index else {
            return;
        };
        let Some(session) = self.state.sessions.get(idx) else {
            return;
        };

        let sid = session.id.clone();
        if self.state.current_session_id.as_deref() == Some(&sid) {
            // Already on this session.
            self.state.set_focus(PanelFocus::Input);
            return;
        }

        self.state.current_session_id = Some(sid.clone());
        let session_id = y_core::types::SessionId::from_string(sid);
        self.load_session_transcript(&session_id).await;

        // Switch focus to input after selecting a session.
        self.state.set_focus(PanelFocus::Input);
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
    // Past end of line — clamp to character count.
    text.chars().count()
}

/// Check if a point (x, y) is inside a `Rect`.
fn contains(rect: ratatui::layout::Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

/// Ensure terminal is restored even if `TuiApp` is dropped without calling
/// `restore_terminal` (e.g., on panic).
impl Drop for TuiApp {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}
