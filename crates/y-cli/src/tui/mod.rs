//! TUI application shell -- entry point, terminal setup, main event loop.
//!
//! `TuiApp` manages the ratatui terminal lifecycle (raw mode, alternate screen)
//! and drives the render-event-update loop. It delegates rendering to panel
//! modules and key handling to the key dispatcher (both in Phase T3+).
//!
//! NOTE: `dead_code` is allowed at module level because this is a scaffold --
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

use std::fmt::Write as _;
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
use commands::handlers::{self, AsyncCommand, CommandResult};
use events::{AppEvent, EventLoop};
use keys::KeyAction;
use layout::LayoutChunks;
use overlays::command_palette::CommandPaletteState;
use state::{
    AppState, ChatMessage, InteractionMode, MessageRole, PanelFocus, SessionListItem, Toast,
    ToastLevel,
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
        Self::ensure_current_session();

        // Initialize context_window from the default provider's metadata.
        if let Some(meta) = self.services.provider_pool().await.list_metadata().first() {
            self.state.context_window = meta.context_window;
        }

        loop {
            self.draw()?;

            let Some(event) = self.events.next().await else {
                break;
            };

            match event {
                AppEvent::Key(key) => {
                    if self.handle_key_event(key).await {
                        break;
                    }
                }
                AppEvent::Mouse(mouse) => self.handle_mouse_event(mouse),
                AppEvent::Resize(_w, _h) => {}
                AppEvent::Tick => self.handle_tick(),
            }
        }

        self.restore_terminal()?;
        Ok(())
    }

    /// Process a key event. Returns `true` when the app should quit.
    async fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> bool {
        let action = keys::dispatch(key, &self.state);
        match action {
            KeyAction::Quit => return true,
            KeyAction::Submit => {
                if self.state.mode == InteractionMode::Command {
                    let cmd_input = if let Some(selected) = self.palette.selected_command() {
                        selected.to_string()
                    } else {
                        self.palette.input.clone()
                    };
                    if self.execute_command(&cmd_input).await {
                        return true;
                    }
                    self.palette = CommandPaletteState::new();
                    self.state.set_mode(InteractionMode::Normal);
                    self.state.set_focus(PanelFocus::Input);
                } else {
                    let input: String = self.textarea.lines().join("\n");
                    let trimmed = input.trim();
                    if trimmed.starts_with('/') && !trimmed.is_empty() {
                        let cmd_input = &trimmed[1..];
                        self.state.push_history(trimmed);
                        if self.execute_command(cmd_input).await {
                            return true;
                        }
                    } else if !trimmed.is_empty() {
                        self.state.push_history(trimmed);
                        self.llm_rx =
                            chat_flow::submit_message(&input, &mut self.state, &self.services);
                    }
                    self.textarea = TextArea::default();
                }
            }
            KeyAction::InputPassthrough => {
                if self.state.mode == InteractionMode::Command {
                    if let crossterm::event::KeyCode::Char(ch) = key.code {
                        self.palette.push_char(ch);
                    } else if key.code == crossterm::event::KeyCode::Backspace {
                        self.palette.pop_char();
                    }
                } else if key.code == crossterm::event::KeyCode::Char('/')
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
            KeyAction::ToggleSidebar => self.state.toggle_sidebar(),
            KeyAction::ToggleSidebarView => self.state.toggle_sidebar_view(),
            KeyAction::CycleFocus => {
                self.state.cycle_focus_forward();
            }
            KeyAction::ScrollUp => {
                if self.state.mode == InteractionMode::Command {
                    self.palette.select_prev();
                } else if self.state.focus == PanelFocus::Sidebar {
                    self.state.select_session_prev();
                } else {
                    self.state.scroll_offset = self.state.scroll_offset.saturating_add(3);
                }
            }
            KeyAction::ScrollDown => {
                if self.state.mode == InteractionMode::Command {
                    self.palette.select_next();
                } else if self.state.focus == PanelFocus::Sidebar {
                    self.state.select_session_next();
                } else {
                    self.state.scroll_offset = self.state.scroll_offset.saturating_sub(3);
                }
            }
            KeyAction::PageScrollUp => {
                let page = self.state.page_height.max(1);
                self.state.scroll_offset = self.state.scroll_offset.saturating_add(page);
            }
            KeyAction::PageScrollDown => {
                let page = self.state.page_height.max(1);
                self.state.scroll_offset = self.state.scroll_offset.saturating_sub(page);
            }
            KeyAction::ScrollToTop => {
                // Set a very large offset to scroll to the beginning.
                self.state.scroll_offset = usize::MAX / 2;
            }
            KeyAction::ScrollToBottom => {
                self.state.scroll_offset = 0;
            }
            KeyAction::CancelStreaming => {
                chat_flow::cancel_streaming(&mut self.state);
                self.llm_rx = None;
                self.state
                    .push_toast("Response cancelled.".into(), ToastLevel::Info);
            }
            KeyAction::ShowHelp => {
                if self.state.mode == InteractionMode::Help {
                    self.state.set_mode(InteractionMode::Normal);
                } else {
                    // Close any other overlay first, then show help.
                    if self.state.mode != InteractionMode::Normal {
                        self.state.set_mode(InteractionMode::Normal);
                    }
                    self.state.set_mode(InteractionMode::Help);
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
        false
    }

    /// Process a mouse event.
    fn handle_mouse_event(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(ref chunks) = self.last_chunks {
                    let (x, y) = (mouse.column, mouse.row);
                    if contains(chunks.input, x, y) {
                        self.state.set_focus(PanelFocus::Input);
                        self.state.selection.reset();
                    } else if contains(chunks.chat, x, y) {
                        self.state.set_focus(PanelFocus::Chat);
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
                if self.state.selection.active {
                    if let Some(ref chunks) = self.last_chunks {
                        let (row, col) =
                            self.terminal_to_content(mouse.column, mouse.row, chunks.chat);
                        self.state.selection.update(row, col);
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.state.selection.active {
                    if let Some(ref chunks) = self.last_chunks {
                        let (row, col) =
                            self.terminal_to_content(mouse.column, mouse.row, chunks.chat);
                        self.state.selection.update(row, col);
                    }
                    self.state.selection.finish();

                    if !self.state.selection.is_empty() {
                        let text =
                            selection::extract_text(&self.chat_plain_lines, &self.state.selection);
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
                    self.state.scroll_offset = self.state.scroll_offset.saturating_add(3);
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
                    self.state.scroll_offset = self.state.scroll_offset.saturating_sub(3);
                }
            }
            _ => {}
        }
    }

    /// Handle periodic tick: drain channels and update timers.
    fn handle_tick(&mut self) {
        self.state.tick_animation();

        if let Some(ref mut rx) = self.toast_rx {
            while let Ok(toast) = rx.try_recv() {
                self.state.push_toast(toast.message, toast.level);
            }
        }

        self.state.tick_toasts();

        if let Some(ref mut rx) = self.llm_rx {
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

    /// Draw the current frame.
    fn draw(&mut self) -> Result<()> {
        // Pre-compute input height and layout outside the closure so we
        // can store them for mouse hit-testing.
        let input_lines = panels::input::input_height(&self.textarea);
        let term_size = self.terminal.size()?;
        let term_rect = ratatui::layout::Rect::new(0, 0, term_size.width, term_size.height);
        let chunks = layout::compute_layout(term_rect, self.state.sidebar_visible, input_lines);
        self.last_chunks = Some(chunks.clone());

        // Update page_height from the chat panel for page-scroll calculations.
        self.state.page_height = chunks.chat.height.saturating_sub(2) as usize;

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

            let pl = Self::render_panels(frame, chunks, state, textarea);
            *plain_lines_cell.borrow_mut() = pl;

            // Render command palette overlay if in Command mode.
            if state.mode == InteractionMode::Command {
                overlays::command_palette::render(frame, area, palette);
            }

            // Render help overlay if in Help mode.
            if state.mode == InteractionMode::Help {
                overlays::help::render(frame, area);
            }

            // Render toast overlay (always, non-modal).
            overlays::toast::render(frame, area, &state.toasts);
        })?;

        // Cache plain text lines rendered by the chat panel.
        self.chat_plain_lines = plain_lines_cell.into_inner();

        Ok(())
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
                self.state.sync_selected_session_index();
                self.state
                    .push_toast("New session started.".into(), ToastLevel::Info);
            }
            CommandResult::Async(cmd) => {
                self.execute_async_command(cmd).await;
            }
        }
        false
    }

    /// Execute an async command that requires service access.
    async fn execute_async_command(&mut self, cmd: AsyncCommand) {
        match cmd {
            AsyncCommand::ListSessions => self.cmd_list_sessions().await,
            AsyncCommand::SwitchSession(target) => self.cmd_switch_session(&target).await,
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
        }
    }

    // -----------------------------------------------------------------------
    // Async command implementations
    // -----------------------------------------------------------------------

    /// `/list` -- list all sessions.
    async fn cmd_list_sessions(&mut self) {
        use y_core::session::SessionFilter;

        match self
            .services
            .session_manager
            .list_sessions(&SessionFilter::default())
            .await
        {
            Ok(mut nodes) => {
                nodes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
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
        use y_core::session::SessionFilter;

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

        // Find by ID prefix or title substring.
        let target_lower = target.to_lowercase();
        let matched = nodes.iter().find(|n| {
            n.id.to_string().starts_with(target)
                || n.title
                    .as_ref()
                    .is_some_and(|t| t.to_lowercase().contains(&target_lower))
        });

        match matched {
            Some(node) => {
                let sid = node.id.clone();
                let title = node
                    .title
                    .clone()
                    .unwrap_or_else(|| sid.to_string()[..8].to_string());
                self.state.current_session_id = Some(sid.to_string());
                self.load_session_transcript(&sid).await;
                self.state.sync_selected_session_index();
                self.state.set_focus(PanelFocus::Input);
                self.state
                    .push_toast(format!("Switched to: {title}"), ToastLevel::Info);
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

        let target_lower = target.to_lowercase();
        let matched = nodes.iter().find(|n| {
            n.id.to_string().starts_with(target)
                || n.title
                    .as_ref()
                    .is_some_and(|t| t.to_lowercase().contains(&target_lower))
        });

        match matched {
            Some(node) => {
                let sid = node.id.clone();
                let is_current = self.state.current_session_id.as_deref() == Some(&sid.to_string());

                match self.services.session_manager.delete_session(&sid).await {
                    Ok(()) => {
                        self.state.push_toast(
                            format!("Deleted session: {}", &sid.to_string()[..8]),
                            ToastLevel::Info,
                        );
                        // Refresh sidebar.
                        self.load_sessions().await;
                        // If we deleted the current session, clear the chat.
                        if is_current {
                            self.state.messages.clear();
                            self.state.current_session_id = None;
                            self.state.user_message_count = 0;
                        }
                        self.state.sync_selected_session_index();
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
                self.state.current_session_id = Some(fork_id.clone());
                self.load_session_transcript(&fork.id).await;
                self.load_sessions().await;
                self.state.sync_selected_session_index();
                self.state.set_focus(PanelFocus::Input);
                self.state
                    .push_toast(format!("Branched: {fork_title}"), ToastLevel::Info);
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

        #[cfg(feature = "tui")]
        {
            match arboard::Clipboard::new() {
                Ok(mut clipboard) => match clipboard.set_text(&content) {
                    Ok(()) => {
                        self.state.push_toast(
                            format!(
                                "Exported {} messages ({fmt}) to clipboard.",
                                self.state.messages.len()
                            ),
                            ToastLevel::Info,
                        );
                    }
                    Err(e) => {
                        self.state
                            .push_toast(format!("Clipboard error: {e}"), ToastLevel::Error);
                    }
                },
                Err(e) => {
                    self.state
                        .push_toast(format!("Clipboard error: {e}"), ToastLevel::Error);
                }
            }
        }

        #[cfg(not(feature = "tui"))]
        {
            let _ = content;
            self.state.push_toast(
                "Clipboard not available without TUI feature.".into(),
                ToastLevel::Error,
            );
        }
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
                        self.state.status_model = meta.model.clone();
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

        // Chat panel -- returns plain text lines for selection.
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
    /// the most recent session -- instead, we leave `current_session_id = None`
    /// so the user always starts with a clean slate. The session will be created
    /// lazily when the first message is sent (see `chat_flow::submit_message`).
    fn ensure_current_session() {
        // Nothing to do -- lazy creation on first message.
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
                        reasoning_content: String::new(),
                        reasoning_complete: false,
                        tool_calls: Vec::new(),
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
    // Past end of line -- clamp to character count.
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
