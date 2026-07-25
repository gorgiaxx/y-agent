//! TUI application state model.
//!
//! Contains all state types used by the TUI: panel focus, interaction mode,
//! chat messages, and the top-level `AppState`.

use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Utc};

use crate::tui::selection::TextSelection;
use crate::tui::theme::Theme;

// TurnMeta
#[derive(Debug, Clone, PartialEq)]
pub struct TurnMeta {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub context_window: usize,
    pub context_tokens_used: u64,
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Which panel currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    /// The input area at the bottom (default).
    Input,
    /// The chat message history panel.
    Chat,
}

/// The current interaction mode — determines how keystrokes are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionMode {
    /// Default mode: typing goes to input area.
    Normal,
    /// Slash-command mode: `/` was typed, command palette visible.
    Command,
    /// Search mode: incremental search overlay visible.
    Search,
    /// Select mode: choosing an earlier user prompt for a non-destructive branch.
    Select,
    /// Help mode: help overlay is visible.
    Help,
    /// Full-screen copy target selector.
    Copy,
    /// Full-screen session resume selector.
    Resume,
    /// Full-screen session prompt-template selector.
    Prompt,
}

/// Service-owned orchestration mode selected by the TUI operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnMode {
    Fast,
    Auto,
    Plan,
    Loop,
}

impl TurnMode {
    /// Parse a user-facing mode name.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fast" => Some(Self::Fast),
            "auto" => Some(Self::Auto),
            "plan" => Some(Self::Plan),
            "loop" => Some(Self::Loop),
            _ => None,
        }
    }

    /// Compact label for status and command output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Auto => "auto",
            Self::Plan => "plan",
            Self::Loop => "loop",
        }
    }

    /// Value expected by the shared service orchestration contract.
    pub fn plan_mode(self) -> Option<&'static str> {
        match self {
            Self::Fast => None,
            Self::Auto => Some("auto"),
            Self::Plan => Some("plan"),
            Self::Loop => Some("loop"),
        }
    }
}

/// Prompt-template state currently persisted for the active session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PromptTemplateStatus {
    /// Built-in prompt composition with no session override.
    #[default]
    Default,
    /// A named user prompt template is active.
    Template { id: String, name: String },
    /// A custom session prompt exists without a template identity.
    Custom,
}

impl PromptTemplateStatus {
    /// Compact label for the TUI status bar.
    pub fn label(&self) -> String {
        match self {
            Self::Default => "prompt:default".to_string(),
            Self::Template { name, .. } => format!("prompt:{name}"),
            Self::Custom => "prompt:custom".to_string(),
        }
    }

    /// Active template ID, if the prompt came from a named template.
    pub fn template_id(&self) -> Option<&str> {
        match self {
            Self::Template { id, .. } => Some(id),
            Self::Default | Self::Custom => None,
        }
    }
}

/// Role of a chat message for display purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

// ---------------------------------------------------------------------------
// ChatMessage
// ---------------------------------------------------------------------------

/// Structured record of an executed tool call for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallStatus {
    Running,
    Succeeded,
    Failed,
}

/// Amount of detail rendered for a tool call card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallDisplayMode {
    Collapsed,
    Preview,
    Expanded,
}

/// Address of a tool call inside the visible transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSelection {
    pub message_index: usize,
    pub tool_index: usize,
}

/// Structured record of a tool call for rendering and copy selection.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallInfo {
    /// Stable service-owned tool call correlation ID.
    pub tool_call_id: String,
    /// Tool name (e.g. "`WebSearch`").
    pub name: String,
    /// Current execution status.
    pub status: ToolCallStatus,
    /// Execution duration in milliseconds once complete.
    pub duration_ms: Option<u64>,
    /// Bounded serialized tool arguments supplied by the service.
    pub input_preview: String,
    /// Bounded tool result supplied by the service.
    pub result_preview: String,
    /// Agent responsible for the call.
    pub agent_name: String,
    /// Optional URL metadata for browser and web tools.
    pub url_meta: Option<String>,
    /// Optional tool-specific structured metadata.
    pub metadata: Option<serde_json::Value>,
    /// Current detail level selected by the operator.
    pub display_mode: ToolCallDisplayMode,
}

impl ToolCallInfo {
    pub fn is_complete(&self) -> bool {
        self.status != ToolCallStatus::Running
    }

    pub fn cycle_display_mode(&mut self) -> ToolCallDisplayMode {
        self.display_mode = match self.display_mode {
            ToolCallDisplayMode::Collapsed => ToolCallDisplayMode::Preview,
            ToolCallDisplayMode::Preview => ToolCallDisplayMode::Expanded,
            ToolCallDisplayMode::Expanded => ToolCallDisplayMode::Collapsed,
        };
        self.display_mode
    }
}

/// A segment in the event-ordered display stream.
///
/// During streaming, `StreamDelta` and `ToolCallExecuted` events arrive
/// interleaved. This enum preserves that ordering so the renderer can
/// display tool call cards at the correct position (between the text
/// chunks that surround them), matching the GUI's event-ordered model.
#[derive(Debug, Clone)]
pub enum StreamSegment {
    /// Accumulated text content (one or more `StreamDelta` events merged).
    Text(String),
    /// Accumulated reasoning content at its event-ordered timeline position.
    Reasoning { content: String, is_complete: bool },
    /// Index into the owning message's `tool_calls` vector.
    ToolCall(usize),
}

/// A single message in the conversation transcript (display model).
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// Who sent the message.
    pub role: MessageRole,
    /// The text content.
    pub content: String,
    /// When the message was created.
    pub timestamp: DateTime<Utc>,
    /// Whether this message is still being streamed.
    pub is_streaming: bool,
    /// Whether this message was cancelled mid-stream.
    pub is_cancelled: bool,
    /// Aggregate reasoning content retained for historical-message compatibility.
    pub reasoning_content: String,
    /// Whether the reasoning phase is complete.
    pub reasoning_complete: bool,
    /// Tool calls executed during this message's generation (structured).
    pub tool_calls: Vec<ToolCallInfo>,
    /// Event-ordered display segments for interleaved rendering.
    ///
    /// Populated during streaming to preserve the arrival order of text
    /// deltas and tool call events. Empty for historical messages (the
    /// renderer falls back to `content` + `tool_calls` parsing).
    pub segments: Vec<StreamSegment>,
}

impl ChatMessage {
    /// Create a new non-streaming message with default fields.
    pub fn system(content: String) -> Self {
        Self {
            role: MessageRole::System,
            content,
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// SessionListItem
// ---------------------------------------------------------------------------

/// Lightweight session entry for command palette completion and session labels.
#[derive(Debug, Clone)]
pub struct SessionListItem {
    /// Session ID.
    pub id: String,
    /// Human-readable title (empty if not yet generated).
    pub title: String,
    /// When the session was last active.
    pub updated_at: DateTime<Utc>,
    /// Total messages in the session.
    pub message_count: u32,
}

// ---------------------------------------------------------------------------
// Toast
// ---------------------------------------------------------------------------

/// Maximum number of concurrent toasts displayed.
pub const MAX_TOASTS: usize = 5;

/// Severity/style of a toast notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    /// Informational message (cyan).
    Info,
    /// Success confirmation (green).
    Success,
    /// Warning (yellow).
    Warning,
    /// Error (red).
    Error,
}

impl ToastLevel {
    /// Default number of ticks (at 250ms each) before auto-dismiss.
    pub fn default_ticks(self) -> u16 {
        match self {
            Self::Info | Self::Success => 12, // 3 seconds
            Self::Warning => 20,              // 5 seconds
            Self::Error => 28,                // 7 seconds
        }
    }
}

/// A transient notification displayed as an overlay in the TUI.
#[derive(Debug, Clone)]
pub struct Toast {
    /// The notification message.
    pub message: String,
    /// Severity level (determines color and duration).
    pub level: ToastLevel,
    /// Ticks remaining before auto-dismiss (decremented each 250ms tick).
    pub ticks_remaining: u16,
    /// Unique ID for targeted dismissal.
    pub id: u64,
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Full TUI application state.
///
/// Owned by `TuiApp` and mutated in response to key events and server events.
/// Rendering reads this immutably each frame.
#[derive(Debug)]
pub struct AppState {
    /// Which panel has keyboard focus.
    pub focus: PanelFocus,
    /// Current interaction mode (determines key dispatch).
    pub mode: InteractionMode,
    /// Conversation transcript for the current session.
    pub messages: Vec<ChatMessage>,
    /// Current input buffer text.
    pub input_buffer: String,
    /// Scroll offset in the chat panel (0 = at bottom).
    pub scroll_offset: usize,
    /// Whether the assistant is currently streaming a response.
    pub is_streaming: bool,
    /// Whether cancellation has been requested for the active response.
    pub is_cancelling: bool,
    /// Input history for up/down recall.
    pub input_history: Vec<String>,
    /// Current index in input history (-1 = not browsing).
    pub history_index: Option<usize>,
    /// Draft input saved when entering history navigation (restored on exit).
    pub input_draft: Option<String>,
    /// Currently active model name (displayed in status bar).
    pub status_model: String,
    /// Token usage string (displayed in status bar).
    pub status_tokens: String,
    /// Orchestration mode used for subsequent turns in this session.
    pub turn_mode: TurnMode,
    /// Active per-session prompt-template state.
    pub prompt_template_status: PromptTemplateStatus,
    /// Active toast notifications (most recent at back).
    pub toasts: VecDeque<Toast>,
    /// Monotonic counter for unique toast IDs.
    toast_counter: u64,
    /// Current text selection in the chat panel.
    pub selection: TextSelection,
    /// Tool card selected for expansion or direct copy.
    pub selected_tool: Option<ToolSelection>,
    /// Transcript index of the user prompt selected for backtracking.
    pub selected_message: Option<usize>,
    /// Recent session list (sorted by `updated_at` desc).
    pub sessions: Vec<SessionListItem>,
    /// Active session ID for the current chat.
    pub current_session_id: Option<String>,
    /// Count of user messages sent in the current session (for title trigger).
    pub user_message_count: u32,
    /// Maximum context window size (tokens) of the active provider.
    pub context_window: usize,
    /// Cumulative input tokens consumed in the current session.
    pub cumulative_input_tokens: u64,
    /// Cumulative output tokens consumed in the current session.
    pub cumulative_output_tokens: u64,
    /// Input tokens from the last LLM iteration (actual context occupancy).
    pub last_input_tokens: u64,
    /// Cost (USD) from the last LLM turn, if available.
    pub last_cost: Option<f64>,
    /// Application version string (e.g. "0.1.0").
    pub version: String,
    /// Visible chat panel height in lines (updated each frame from layout).
    pub page_height: usize,
    /// User-selected provider ID for the next turn. None = auto (pool assigns).
    pub selected_provider_id: Option<String>,
    /// Monotonic tick counter for frame-based animations (incremented every 250ms).
    pub tick_counter: u64,
    pub turn_meta_cache: HashMap<String, TurnMeta>,
    /// Terminal-aware color theme (auto-detected at startup).
    pub theme: Theme,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            focus: PanelFocus::Input,
            mode: InteractionMode::Normal,
            messages: Vec::new(),
            input_buffer: String::new(),
            scroll_offset: 0,
            is_streaming: false,
            is_cancelling: false,
            input_history: Vec::new(),
            history_index: None,
            input_draft: None,
            status_model: String::new(),
            status_tokens: String::new(),
            turn_mode: TurnMode::Fast,
            prompt_template_status: PromptTemplateStatus::Default,
            toasts: VecDeque::new(),
            toast_counter: 0,
            selection: TextSelection::default(),
            selected_tool: None,
            selected_message: None,
            sessions: Vec::new(),
            current_session_id: None,
            user_message_count: 0,
            context_window: 0,
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            last_input_tokens: 0,
            last_cost: None,
            version: env!("CARGO_PKG_VERSION").to_string(),
            page_height: 20,
            selected_provider_id: None,
            tick_counter: 0,
            turn_meta_cache: HashMap::new(),
            theme: Theme::default(),
        }
    }
}

impl AppState {
    /// Create a new `AppState` with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the panel focus.
    pub fn set_focus(&mut self, focus: PanelFocus) {
        self.focus = focus;
    }

    /// Set the interaction mode.
    ///
    /// Enforces the design state machine:
    /// - `Normal` → `Command` | `Search` | `Select` | picker modes
    /// - `Command` → `Normal`
    /// - `Search` → `Normal`
    /// - `Select` and picker modes → `Normal`
    ///
    /// Returns `true` if the transition was accepted.
    pub fn set_mode(&mut self, mode: InteractionMode) -> bool {
        let valid = matches!(
            (self.mode, mode),
            // From Normal to any other mode
            (
                InteractionMode::Normal,
                InteractionMode::Command
                    | InteractionMode::Search
                    | InteractionMode::Select
                    | InteractionMode::Help
                    | InteractionMode::Copy
                    | InteractionMode::Resume
                    | InteractionMode::Prompt
                    | InteractionMode::Normal
            ) | (
                InteractionMode::Command
                    | InteractionMode::Search
                    | InteractionMode::Select
                    | InteractionMode::Help
                    | InteractionMode::Copy
                    | InteractionMode::Resume
                    | InteractionMode::Prompt,
                InteractionMode::Normal
            ) | (
                InteractionMode::Command,
                InteractionMode::Copy | InteractionMode::Resume | InteractionMode::Prompt
            )
        );

        if valid {
            self.mode = mode;
        }
        valid
    }

    /// Push a toast notification. Returns the assigned toast ID.
    ///
    /// If the toast queue exceeds `MAX_TOASTS`, the oldest toast is evicted.
    pub fn push_toast(&mut self, message: String, level: ToastLevel) -> u64 {
        self.toast_counter += 1;
        let id = self.toast_counter;
        self.toasts.push_back(Toast {
            message,
            level,
            ticks_remaining: level.default_ticks(),
            id,
        });
        if self.toasts.len() > MAX_TOASTS {
            self.toasts.pop_front();
        }
        id
    }

    /// Decrement all toast timers and remove expired toasts.
    ///
    /// Called once per tick (250ms) in the event loop.
    pub fn tick_toasts(&mut self) {
        for toast in &mut self.toasts {
            toast.ticks_remaining = toast.ticks_remaining.saturating_sub(1);
        }
        self.toasts.retain(|t| t.ticks_remaining > 0);
    }

    /// Dismiss a specific toast by ID.
    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }

    /// Dismiss all active toasts.
    pub fn dismiss_all_toasts(&mut self) {
        self.toasts.clear();
    }

    /// Increment user message counter and return the new count.
    pub fn increment_user_message_count(&mut self) -> u32 {
        self.user_message_count += 1;
        self.user_message_count
    }

    /// Compute context window usage percentage based on the last LLM call's
    /// input tokens versus the provider's context window limit.
    ///
    /// Uses `last_input_tokens` (the single latest iteration) rather than
    /// the cumulative total so the status bar reflects actual context
    /// occupancy for the current prompt window.
    ///
    /// Returns 0.0 if `context_window` is 0 (unknown).
    pub fn context_usage_percent(&self) -> f32 {
        if self.context_window == 0 {
            return 0.0;
        }
        (self.last_input_tokens as f32 / self.context_window as f32) * 100.0
    }

    /// Cycle focus forward: Input → Chat → Input.
    /// Returns the new focus.
    pub fn cycle_focus_forward(&mut self) -> PanelFocus {
        self.focus = match self.focus {
            PanelFocus::Input => PanelFocus::Chat,
            PanelFocus::Chat => PanelFocus::Input,
        };
        self.focus
    }

    /// Select the next tool call in transcript order.
    pub fn select_next_tool(&mut self) -> Option<ToolSelection> {
        let positions = self.tool_positions();
        if positions.is_empty() {
            self.selected_tool = None;
            return None;
        }
        let next_index = self
            .selected_tool
            .and_then(|selected| positions.iter().position(|position| *position == selected))
            .map_or(0, |index| (index + 1) % positions.len());
        let selection = positions[next_index];
        self.selected_tool = Some(selection);
        Some(selection)
    }

    /// Select the previous tool call in transcript order.
    pub fn select_previous_tool(&mut self) -> Option<ToolSelection> {
        let positions = self.tool_positions();
        if positions.is_empty() {
            self.selected_tool = None;
            return None;
        }
        let previous_index = self
            .selected_tool
            .and_then(|selected| positions.iter().position(|position| *position == selected))
            .map_or(positions.len() - 1, |index| {
                index.checked_sub(1).unwrap_or(positions.len() - 1)
            });
        let selection = positions[previous_index];
        self.selected_tool = Some(selection);
        Some(selection)
    }

    /// Return the currently selected tool call if the selection remains valid.
    pub fn selected_tool(&self) -> Option<&ToolCallInfo> {
        let selection = self.selected_tool?;
        self.messages
            .get(selection.message_index)?
            .tool_calls
            .get(selection.tool_index)
    }

    /// Cycle the selected tool between preview, expanded, and collapsed modes.
    pub fn cycle_selected_tool_display(&mut self) -> Option<ToolCallDisplayMode> {
        let selection = self.selected_tool?;
        self.messages
            .get_mut(selection.message_index)?
            .tool_calls
            .get_mut(selection.tool_index)
            .map(ToolCallInfo::cycle_display_mode)
    }

    fn tool_positions(&self) -> Vec<ToolSelection> {
        self.messages
            .iter()
            .enumerate()
            .flat_map(|(message_index, message)| {
                (0..message.tool_calls.len()).map(move |tool_index| ToolSelection {
                    message_index,
                    tool_index,
                })
            })
            .collect()
    }

    /// Begin backtrack selection at the most recent user prompt.
    pub fn begin_backtrack_selection(&mut self) -> Option<usize> {
        let selected = self.user_message_positions().last().copied();
        self.selected_message = selected;
        selected
    }

    /// Move backtrack selection to the previous user prompt, clamping at the oldest.
    pub fn select_previous_user_message(&mut self) -> Option<usize> {
        let positions = self.user_message_positions();
        if positions.is_empty() {
            self.selected_message = None;
            return None;
        }

        let current_position = self
            .selected_message
            .and_then(|selected| positions.iter().position(|position| *position == selected))
            .unwrap_or(positions.len() - 1);
        let selected = positions[current_position.saturating_sub(1)];
        self.selected_message = Some(selected);
        Some(selected)
    }

    /// Move backtrack selection to the next user prompt, clamping at the newest.
    pub fn select_next_user_message(&mut self) -> Option<usize> {
        let positions = self.user_message_positions();
        if positions.is_empty() {
            self.selected_message = None;
            return None;
        }

        let current_position = self
            .selected_message
            .and_then(|selected| positions.iter().position(|position| *position == selected))
            .unwrap_or(positions.len() - 1);
        let selected = positions[(current_position + 1).min(positions.len() - 1)];
        self.selected_message = Some(selected);
        Some(selected)
    }

    /// Return the selected user prompt if the transcript has not invalidated it.
    pub fn selected_user_message(&self) -> Option<&ChatMessage> {
        self.selected_message
            .and_then(|index| self.messages.get(index))
            .filter(|message| message.role == MessageRole::User)
    }

    /// Clear any active prompt backtrack selection.
    pub fn clear_backtrack_selection(&mut self) {
        self.selected_message = None;
    }

    fn user_message_positions(&self) -> Vec<usize> {
        self.messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| (message.role == MessageRole::User).then_some(index))
            .collect()
    }

    /// Label the active session for compact status rendering.
    pub fn current_session_label(&self) -> String {
        let Some(current_id) = self.current_session_id.as_deref() else {
            return "new session".to_string();
        };

        self.sessions
            .iter()
            .find(|session| session.id == current_id)
            .map_or_else(
                || short_session_id(current_id),
                |session| {
                    if session.title.trim().is_empty() {
                        short_session_id(current_id)
                    } else {
                        session.title.clone()
                    }
                },
            )
    }

    /// Push a non-empty, non-duplicate entry to input history.
    pub fn push_history(&mut self, input: &str) {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return;
        }
        // Avoid consecutive duplicates.
        if self.input_history.last().map(std::string::String::as_str) == Some(trimmed) {
            return;
        }
        self.input_history.push(trimmed.to_string());
        self.history_index = None;
        self.input_draft = None;
    }

    /// Navigate to the previous history entry. Returns the entry text.
    pub fn history_prev(&mut self) -> Option<&str> {
        if self.input_history.is_empty() {
            return None;
        }
        let idx = match self.history_index {
            Some(i) => i.saturating_sub(1),
            None => self.input_history.len() - 1,
        };
        self.history_index = Some(idx);
        self.input_history.get(idx).map(std::string::String::as_str)
    }

    /// Increment the animation tick counter. Called once per event-loop tick.
    pub fn tick_animation(&mut self) {
        self.tick_counter = self.tick_counter.wrapping_add(1);
    }

    /// Navigate to the next history entry. Returns `None` when past the end
    /// (meaning "clear the input, return to fresh typing").
    pub fn history_next(&mut self) -> Option<&str> {
        match self.history_index {
            Some(i) => {
                let next = i + 1;
                if next >= self.input_history.len() {
                    self.history_index = None;
                    None
                } else {
                    self.history_index = Some(next);
                    self.input_history
                        .get(next)
                        .map(std::string::String::as_str)
                }
            }
            None => None,
        }
    }
}

fn short_session_id(session_id: &str) -> String {
    session_id.chars().take(8).collect()
}

// ---------------------------------------------------------------------------
// Tests — TDD: these were written FIRST per the test plan.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // T-TUI-01-01: AppState initializes with Input focus and Normal mode.
    #[test]
    fn test_app_state_default_focus_and_mode() {
        let state = AppState::new();
        assert_eq!(state.focus, PanelFocus::Input);
        assert_eq!(state.mode, InteractionMode::Normal);
        assert!(state.messages.is_empty());
        assert!(state.input_buffer.is_empty());
        assert_eq!(state.scroll_offset, 0);
        assert!(!state.is_streaming);
        assert!(!state.is_cancelling);
        assert_eq!(state.turn_mode, TurnMode::Fast);
        assert_eq!(state.prompt_template_status, PromptTemplateStatus::Default);
        assert_eq!(state.prompt_template_status.label(), "prompt:default");
    }

    #[test]
    fn test_prompt_template_status_labels_template_and_custom_configs() {
        let template = PromptTemplateStatus::Template {
            id: "daily-driver".into(),
            name: "Daily Driver".into(),
        };
        assert_eq!(template.label(), "prompt:Daily Driver");
        assert_eq!(template.template_id(), Some("daily-driver"));
        assert_eq!(PromptTemplateStatus::Custom.label(), "prompt:custom");
        assert_eq!(PromptTemplateStatus::Default.template_id(), None);
    }

    // T-TUI-01-02: set_focus() transitions update focus field.
    #[test]
    fn test_set_focus_transitions() {
        let mut state = AppState::new();

        state.set_focus(PanelFocus::Chat);
        assert_eq!(state.focus, PanelFocus::Chat);

        state.set_focus(PanelFocus::Input);
        assert_eq!(state.focus, PanelFocus::Input);
    }

    // T-TUI-01-03: set_mode() transitions update mode field.
    #[test]
    fn test_set_mode_transitions() {
        let mut state = AppState::new();

        // Normal → Command
        assert!(state.set_mode(InteractionMode::Command));
        assert_eq!(state.mode, InteractionMode::Command);

        // Command → Normal
        assert!(state.set_mode(InteractionMode::Normal));
        assert_eq!(state.mode, InteractionMode::Normal);

        // Normal → Search
        assert!(state.set_mode(InteractionMode::Search));
        assert_eq!(state.mode, InteractionMode::Search);

        // Search → Normal
        assert!(state.set_mode(InteractionMode::Normal));
        assert_eq!(state.mode, InteractionMode::Normal);

        // Normal → Select
        assert!(state.set_mode(InteractionMode::Select));
        assert_eq!(state.mode, InteractionMode::Select);

        // Select → Normal
        assert!(state.set_mode(InteractionMode::Normal));
        assert_eq!(state.mode, InteractionMode::Normal);

        // Normal → Prompt picker → Normal
        assert!(state.set_mode(InteractionMode::Prompt));
        assert_eq!(state.mode, InteractionMode::Prompt);
        assert!(state.set_mode(InteractionMode::Normal));
        assert_eq!(state.mode, InteractionMode::Normal);
    }

    // T-TUI-01-05: InteractionMode state transitions follow design state machine.
    #[test]
    fn test_mode_state_machine_rejects_invalid() {
        let mut state = AppState::new();

        // Command → Search (invalid: must go through Normal)
        state.mode = InteractionMode::Command;
        assert!(!state.set_mode(InteractionMode::Search));
        assert_eq!(state.mode, InteractionMode::Command); // unchanged

        // Command → Select (invalid)
        assert!(!state.set_mode(InteractionMode::Select));
        assert_eq!(state.mode, InteractionMode::Command);

        // Search → Command (invalid)
        state.mode = InteractionMode::Search;
        assert!(!state.set_mode(InteractionMode::Command));
        assert_eq!(state.mode, InteractionMode::Search);

        // Search → Select (invalid)
        assert!(!state.set_mode(InteractionMode::Select));
        assert_eq!(state.mode, InteractionMode::Search);

        // Select → Command (invalid)
        state.mode = InteractionMode::Select;
        assert!(!state.set_mode(InteractionMode::Command));
        assert_eq!(state.mode, InteractionMode::Select);

        // Select → Search (invalid)
        assert!(!state.set_mode(InteractionMode::Search));
        assert_eq!(state.mode, InteractionMode::Select);
    }

    // T-TUI-01-06: Panel focus cycles only between input and conversation.
    #[test]
    fn test_focus_cycle_forward() {
        let mut state = AppState::new();

        // Input → Chat
        assert_eq!(state.cycle_focus_forward(), PanelFocus::Chat);

        // Chat → Input
        assert_eq!(state.cycle_focus_forward(), PanelFocus::Input);
    }

    #[test]
    fn test_current_session_label_uses_title_or_short_id() {
        let mut state = AppState::new();
        assert_eq!(state.current_session_label(), "new session");

        state.current_session_id = Some("1234567890abcdef".to_string());
        assert_eq!(state.current_session_label(), "12345678");

        state.sessions.push(SessionListItem {
            id: "1234567890abcdef".to_string(),
            title: "Release work".to_string(),
            updated_at: Utc::now(),
            message_count: 3,
        });
        assert_eq!(state.current_session_label(), "Release work");
    }

    #[test]
    fn test_backtrack_selection_navigates_only_user_messages() {
        let mut state = AppState::new();
        let mut user_one = ChatMessage::system("first prompt".into());
        user_one.role = MessageRole::User;
        let mut assistant = ChatMessage::system("answer".into());
        assistant.role = MessageRole::Assistant;
        let mut user_two = ChatMessage::system("second prompt".into());
        user_two.role = MessageRole::User;
        state.messages = vec![user_one, assistant, user_two];

        assert_eq!(state.begin_backtrack_selection(), Some(2));
        assert_eq!(state.select_previous_user_message(), Some(0));
        assert_eq!(state.select_previous_user_message(), Some(0));
        assert_eq!(state.select_next_user_message(), Some(2));
        assert_eq!(
            state
                .selected_user_message()
                .map(|message| message.content.as_str()),
            Some("second prompt")
        );

        state.clear_backtrack_selection();
        assert_eq!(state.selected_message, None);
    }

    #[test]
    fn test_turn_mode_maps_to_service_plan_mode() {
        assert_eq!(TurnMode::Fast.plan_mode(), None);
        assert_eq!(TurnMode::Auto.plan_mode(), Some("auto"));
        assert_eq!(TurnMode::Plan.plan_mode(), Some("plan"));
        assert_eq!(TurnMode::Loop.plan_mode(), Some("loop"));
    }

    #[test]
    fn test_turn_mode_parses_user_facing_names() {
        assert_eq!(TurnMode::parse("FAST"), Some(TurnMode::Fast));
        assert_eq!(TurnMode::parse("auto"), Some(TurnMode::Auto));
        assert_eq!(TurnMode::parse("plan"), Some(TurnMode::Plan));
        assert_eq!(TurnMode::parse("loop"), Some(TurnMode::Loop));
        assert_eq!(TurnMode::parse("unknown"), None);
    }

    #[test]
    fn test_push_history_dedup() {
        let mut state = AppState::new();
        state.push_history("hello");
        state.push_history("hello");
        assert_eq!(state.input_history.len(), 1);
        state.push_history("world");
        assert_eq!(state.input_history.len(), 2);
    }

    #[test]
    fn test_push_history_ignores_empty() {
        let mut state = AppState::new();
        state.push_history("");
        state.push_history("   ");
        assert!(state.input_history.is_empty());
    }

    #[test]
    fn test_history_prev_next() {
        let mut state = AppState::new();
        state.push_history("first");
        state.push_history("second");
        state.push_history("third");

        assert_eq!(state.history_prev(), Some("third"));
        assert_eq!(state.history_prev(), Some("second"));
        assert_eq!(state.history_prev(), Some("first"));
        // Clamps at beginning.
        assert_eq!(state.history_prev(), Some("first"));

        // Navigate forward.
        assert_eq!(state.history_next(), Some("second"));
        assert_eq!(state.history_next(), Some("third"));
        // Past end returns None (clear input).
        assert_eq!(state.history_next(), None);
    }

    // --- Toast tests ---

    // T-TOAST-01: push_toast adds toast with correct level and default ticks.
    #[test]
    fn test_push_toast_adds_with_defaults() {
        let mut state = AppState::new();
        let id = state.push_toast("hello".into(), ToastLevel::Info);
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].id, id);
        assert_eq!(state.toasts[0].level, ToastLevel::Info);
        assert_eq!(state.toasts[0].ticks_remaining, 12);
        assert_eq!(state.toasts[0].message, "hello");
    }

    // T-TOAST-02: tick_toasts decrements all toasts' ticks_remaining.
    #[test]
    fn test_tick_toasts_decrements() {
        let mut state = AppState::new();
        state.push_toast("a".into(), ToastLevel::Error);
        state.push_toast("b".into(), ToastLevel::Warning);

        let before_a = state.toasts[0].ticks_remaining;
        let before_b = state.toasts[1].ticks_remaining;

        state.tick_toasts();

        assert_eq!(state.toasts[0].ticks_remaining, before_a - 1);
        assert_eq!(state.toasts[1].ticks_remaining, before_b - 1);
    }

    // T-TOAST-03: tick_toasts removes toasts when ticks_remaining reaches 0.
    #[test]
    fn test_tick_toasts_removes_expired() {
        let mut state = AppState::new();
        state.push_toast("ephemeral".into(), ToastLevel::Info);
        // Manually set ticks to 1 so next tick expires it.
        state.toasts[0].ticks_remaining = 1;

        state.tick_toasts();
        assert!(state.toasts.is_empty());
    }

    // T-TOAST-04: Maximum 5 toasts; 6th evicts oldest.
    #[test]
    fn test_toast_max_eviction() {
        let mut state = AppState::new();
        for i in 0..6 {
            state.push_toast(format!("toast {i}"), ToastLevel::Info);
        }
        assert_eq!(state.toasts.len(), MAX_TOASTS);
        // The first toast (id=1) should have been evicted.
        assert_eq!(state.toasts.front().unwrap().id, 2);
        assert_eq!(state.toasts.back().unwrap().id, 6);
    }

    // T-TOAST-05: dismiss_toast removes specific toast by ID.
    #[test]
    fn test_dismiss_toast_by_id() {
        let mut state = AppState::new();
        let id1 = state.push_toast("a".into(), ToastLevel::Info);
        let _id2 = state.push_toast("b".into(), ToastLevel::Warning);

        state.dismiss_toast(id1);
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].message, "b");
    }

    // T-TOAST-06: dismiss_all_toasts clears all toasts.
    #[test]
    fn test_dismiss_all_toasts() {
        let mut state = AppState::new();
        state.push_toast("a".into(), ToastLevel::Info);
        state.push_toast("b".into(), ToastLevel::Error);
        state.dismiss_all_toasts();
        assert!(state.toasts.is_empty());
    }

    // T-TOAST-07: Toast IDs are monotonically increasing.
    #[test]
    fn test_toast_ids_monotonic() {
        let mut state = AppState::new();
        let id1 = state.push_toast("a".into(), ToastLevel::Info);
        let id2 = state.push_toast("b".into(), ToastLevel::Info);
        let id3 = state.push_toast("c".into(), ToastLevel::Info);
        assert!(id2 > id1);
        assert!(id3 > id2);
    }

    // T-TOAST-08: Default tick counts per level.
    #[test]
    fn test_toast_default_ticks() {
        assert_eq!(ToastLevel::Info.default_ticks(), 12);
        assert_eq!(ToastLevel::Success.default_ticks(), 12);
        assert_eq!(ToastLevel::Warning.default_ticks(), 20);
        assert_eq!(ToastLevel::Error.default_ticks(), 28);
    }

    // T-TOAST-09: AppState default has empty toasts.
    #[test]
    fn test_app_state_default_toasts_empty() {
        let state = AppState::new();
        assert!(state.toasts.is_empty());
    }

    // T-CTX-01: context_usage_percent returns 0 when context_window is 0.
    #[test]
    fn test_context_usage_percent_zero_window() {
        let state = AppState::new();
        assert_eq!(state.context_usage_percent(), 0.0);
    }

    // T-CTX-02: context_usage_percent uses last_input_tokens (not cumulative).
    #[test]
    fn test_context_usage_percent_calculation() {
        let mut state = AppState::new();
        state.context_window = 100_000;
        state.last_input_tokens = 50_000;
        // Cumulative should NOT affect the percentage.
        state.cumulative_input_tokens = 200_000;
        let pct = state.context_usage_percent();
        assert!((pct - 50.0).abs() < 0.1, "expected ~50%, got {pct}");
    }

    // T-STATE-TOOL-01: ToolCallInfo construction.
    #[test]
    fn test_tool_call_info_creation() {
        let tc = ToolCallInfo {
            tool_call_id: "call-search-1".into(),
            name: "WebSearch".into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(120),
            input_preview: r#"{"query":"ratatui"}"#.into(),
            result_preview: "3 results".into(),
            agent_name: "chat-turn".into(),
            url_meta: Some("https://example.com".into()),
            metadata: None,
            display_mode: ToolCallDisplayMode::Preview,
        };
        assert_eq!(tc.name, "WebSearch");
        assert_eq!(tc.status, ToolCallStatus::Succeeded);
        assert_eq!(tc.duration_ms, Some(120));
        assert_eq!(tc.input_preview, r#"{"query":"ratatui"}"#);
        assert_eq!(tc.result_preview, "3 results");
        assert!(tc.is_complete());
    }

    #[test]
    fn test_tool_call_display_mode_cycles_all_states() {
        let mut tool = ToolCallInfo {
            tool_call_id: "call-shell-1".into(),
            name: "ShellExec".into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(10),
            input_preview: String::new(),
            result_preview: String::new(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Preview,
        };

        assert_eq!(tool.cycle_display_mode(), ToolCallDisplayMode::Expanded);
        assert_eq!(tool.cycle_display_mode(), ToolCallDisplayMode::Collapsed);
        assert_eq!(tool.cycle_display_mode(), ToolCallDisplayMode::Preview);
    }

    #[test]
    fn test_app_state_selects_tools_in_transcript_order_and_toggles_selected() {
        let mut state = AppState::default();
        let make_tool = |name: &str| ToolCallInfo {
            tool_call_id: format!("call-{name}"),
            name: name.into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(1),
            input_preview: String::new(),
            result_preview: String::new(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Preview,
        };
        let mut first = ChatMessage::system(String::new());
        first.role = MessageRole::Assistant;
        first.tool_calls.push(make_tool("ReadFile"));
        let mut second = ChatMessage::system(String::new());
        second.role = MessageRole::Assistant;
        second.tool_calls.push(make_tool("ShellExec"));
        state.messages = vec![first, second];

        assert_eq!(state.select_next_tool().unwrap().message_index, 0);
        assert_eq!(state.select_next_tool().unwrap().message_index, 1);
        assert_eq!(state.select_previous_tool().unwrap().message_index, 0);
        assert_eq!(
            state.cycle_selected_tool_display(),
            Some(ToolCallDisplayMode::Expanded)
        );
        assert_eq!(
            state.selected_tool().unwrap().display_mode,
            ToolCallDisplayMode::Expanded
        );
    }

    // T-STATE-MSG-01: ChatMessage::system() helper.
    #[test]
    fn test_chat_message_system_helper() {
        let msg = ChatMessage::system("hello".into());
        assert_eq!(msg.role, MessageRole::System);
        assert_eq!(msg.content, "hello");
        assert!(!msg.is_streaming);
        assert!(!msg.is_cancelled);
        assert!(msg.reasoning_content.is_empty());
        assert!(!msg.reasoning_complete);
        assert!(msg.tool_calls.is_empty());
    }

    // T-STATE-PROV-01: selected_provider_id defaults to None.
    #[test]
    fn test_app_state_default_selected_provider_none() {
        let state = AppState::new();
        assert!(state.selected_provider_id.is_none());
    }

    // T-STATE-TICK-01: tick_animation increments counter.
    #[test]
    fn test_app_state_tick_counter_increments() {
        let mut state = AppState::new();
        assert_eq!(state.tick_counter, 0);
        state.tick_animation();
        assert_eq!(state.tick_counter, 1);
        state.tick_animation();
        assert_eq!(state.tick_counter, 2);
    }

    // T-STATE-TICK-02: tick_animation wraps at u64::MAX.
    #[test]
    fn test_app_state_tick_counter_wraps() {
        let mut state = AppState::new();
        state.tick_counter = u64::MAX;
        state.tick_animation();
        assert_eq!(state.tick_counter, 0);
    }

    // T-CTX-03: context_usage_percent caps conceptually (caller clamps).
    #[test]
    fn test_context_usage_percent_over_100() {
        let mut state = AppState::new();
        state.context_window = 1000;
        state.last_input_tokens = 1500;
        let pct = state.context_usage_percent();
        assert!(pct > 100.0, "expected >100%, got {pct}");
    }

    // T-TURNMETA-01: TurnMeta struct construction.
    #[test]
    fn test_turn_meta_creation() {
        let meta = TurnMeta {
            model: "gpt-4o".into(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.002,
            context_window: 128_000,
            context_tokens_used: 100,
        };
        assert_eq!(meta.model, "gpt-4o");
        assert_eq!(meta.input_tokens, 100);
        assert_eq!(meta.output_tokens, 50);
        assert!((meta.cost_usd - 0.002).abs() < f64::EPSILON);
        assert_eq!(meta.context_window, 128_000);
        assert_eq!(meta.context_tokens_used, 100);
    }

    // T-TURNMETA-02: AppState default has empty turn_meta_cache.
    #[test]
    fn test_app_state_default_turn_meta_cache_empty() {
        let state = AppState::new();
        assert!(state.turn_meta_cache.is_empty());
    }

    // T-TURNMETA-03: TurnMeta can be inserted and retrieved from cache.
    #[test]
    fn test_turn_meta_cache_insert_get() {
        let mut state = AppState::new();
        let meta = TurnMeta {
            model: "gpt-4o".into(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.002,
            context_window: 128_000,
            context_tokens_used: 100,
        };
        state
            .turn_meta_cache
            .insert("session-1".into(), meta.clone());
        assert_eq!(state.turn_meta_cache.get("session-1"), Some(&meta));
        assert!(state.turn_meta_cache.get("unknown").is_none());
    }

    // T-TURNMETA-04: TurnMeta cache supports multiple sessions.
    #[test]
    fn test_turn_meta_cache_multiple_sessions() {
        let mut state = AppState::new();
        let meta1 = TurnMeta {
            model: "gpt-4o".into(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.002,
            context_window: 128_000,
            context_tokens_used: 100,
        };
        let meta2 = TurnMeta {
            model: "claude-3".into(),
            input_tokens: 200,
            output_tokens: 100,
            cost_usd: 0.005,
            context_window: 200_000,
            context_tokens_used: 200,
        };
        state.turn_meta_cache.insert("session-1".into(), meta1);
        state.turn_meta_cache.insert("session-2".into(), meta2);
        assert_eq!(state.turn_meta_cache.len(), 2);
        assert_eq!(
            state.turn_meta_cache.get("session-1").unwrap().model,
            "gpt-4o"
        );
        assert_eq!(
            state.turn_meta_cache.get("session-2").unwrap().model,
            "claude-3"
        );
    }
}
