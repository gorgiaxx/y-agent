//! Key dispatcher: maps key events to state transitions based on mode and focus.
//!
//! The dispatcher follows a two-tier priority:
//! 1. **Global keys** (Ctrl+Q/D/C, F1, Ctrl+O) — always handled, regardless of mode/focus.
//! 2. **Mode + Focus keys** — dispatched based on `InteractionMode` × `PanelFocus`.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;
#[cfg(test)]
use std::sync::LazyLock;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde::Deserialize;

use crate::tui::state::{AppState, InteractionMode, PanelFocus};
use crate::tui::terminal::TerminalCapabilities;

/// Result of dispatching a key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Quit the TUI application.
    Quit,
    /// Ask for a second quit gesture before exiting.
    ConfirmQuit,
    /// Clear the current composer draft without exiting.
    ClearInput,
    /// Arm the next terminal paste to bypass fragment collapsing.
    PasteRaw,
    /// Read image media from the system clipboard.
    PasteImage,
    /// Search persisted prompt history.
    OpenHistorySearch,
    /// Search the active session transcript.
    OpenTranscriptSearch,
    /// Edit the current draft in `$VISUAL` or `$EDITOR`.
    OpenExternalEditor,
    /// No-op — the key was consumed but had no effect.
    Consumed,
    /// The key was not handled by the dispatcher.
    Unhandled,
    /// Submit the current input buffer.
    Submit,
    /// A character/edit to pass through to the textarea.
    InputPassthrough,
    /// Cycle focus forward.
    CycleFocus,
    /// Scroll chat up.
    ScrollUp,
    /// Scroll chat down.
    ScrollDown,
    /// Scroll chat up by one page.
    PageScrollUp,
    /// Scroll chat down by one page.
    PageScrollDown,
    /// Scroll to the top of the chat.
    ScrollToTop,
    /// Scroll to the bottom of the chat.
    ScrollToBottom,
    /// Cancel the current streaming response.
    CancelStreaming,
    /// Show the help overlay.
    ShowHelp,
    /// Temporarily expose the transcript in normal terminal scrollback.
    ShowRawScrollback,
    /// Enter command mode.
    EnterCommandMode,
    /// Enter persistent shell mode.
    EnterShellMode,
    /// Return to normal mode.
    ReturnToNormal,
    /// Enter prompt backtrack selection from the current session.
    EnterBacktrack,
    /// Select the previous user prompt in backtrack mode.
    BacktrackPrevious,
    /// Select the next user prompt in backtrack mode.
    BacktrackNext,
    /// Branch before the selected prompt and restore it to the composer.
    ConfirmBacktrack,
    /// Retry the selected prompt in a new branch.
    BacktrackRetry,
    /// Quote the selected prompt into the composer.
    BacktrackQuote,
    /// Fork before the selected prompt without editing it.
    BacktrackFork,
    /// Copy the selected prompt exactly.
    BacktrackCopy,
    /// Focus tools associated with the selected turn.
    BacktrackInspectTools,
    /// Show file-change details associated with the selected turn.
    BacktrackDiff,
    /// Navigate to previous input history entry.
    HistoryPrev,
    /// Navigate to next input history entry.
    HistoryNext,
    /// Select the next tool card in transcript order.
    SelectNextTool,
    /// Select the previous tool card in transcript order.
    SelectPreviousTool,
    /// Cycle the selected tool card detail level.
    ToggleSelectedTool,
    /// Copy the selected tool card.
    CopySelectedTool,
    /// Quote the selected copy target into the composer.
    CopyQuote,
    /// Open a selected filesystem path with the host shell.
    CopyOpenPath,
    /// Remove the selected follow-up from the queue.
    QueueDelete,
    /// Promote the selected follow-up to the pending steer (or demote it).
    QueueSteer,
    /// Remove the selected follow-up and recall it into the composer.
    QueueRecall,
    /// Kill the selected entry in the `/tasks` overlay.
    TasksKill,
    /// Refresh the `/tasks` overlay contents.
    TasksRefresh,
    /// Toggle the focused option in an `AskUser` question.
    AskUserToggle,
    /// Dismiss a pending `AskUser` question without an answer.
    AskUserDismiss,
    /// Dismiss a pending permission or plan-review prompt (deny / reject).
    PermissionDismiss,
    /// Toggle pin on the selected session.
    SessionPin,
    /// Archive the selected session.
    SessionArchive,
    /// Request deletion of the selected session.
    SessionDelete,
    /// Recall a rename command for the selected session.
    SessionRename,
    SessionSlot1,
    SessionSlot2,
    SessionSlot3,
    SessionSlot4,
    SessionSlot5,
}

impl KeyAction {
    /// Stable configuration identifier for this semantic action.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::ConfirmQuit => "confirm_quit",
            Self::ClearInput => "clear_input",
            Self::PasteRaw => "paste_raw",
            Self::PasteImage => "paste_image",
            Self::OpenHistorySearch => "open_history_search",
            Self::OpenTranscriptSearch => "open_transcript_search",
            Self::OpenExternalEditor => "open_external_editor",
            Self::Consumed => "consumed",
            Self::Unhandled => "unhandled",
            Self::Submit => "submit",
            Self::InputPassthrough => "input_passthrough",
            Self::CycleFocus => "cycle_focus",
            Self::ScrollUp => "scroll_up",
            Self::ScrollDown => "scroll_down",
            Self::PageScrollUp => "page_scroll_up",
            Self::PageScrollDown => "page_scroll_down",
            Self::ScrollToTop => "scroll_to_top",
            Self::ScrollToBottom => "scroll_to_bottom",
            Self::CancelStreaming => "cancel_streaming",
            Self::ShowHelp => "show_help",
            Self::ShowRawScrollback => "show_raw_scrollback",
            Self::EnterCommandMode => "enter_command_mode",
            Self::EnterShellMode => "enter_shell_mode",
            Self::ReturnToNormal => "return_to_normal",
            Self::EnterBacktrack => "enter_backtrack",
            Self::BacktrackPrevious => "backtrack_previous",
            Self::BacktrackNext => "backtrack_next",
            Self::ConfirmBacktrack => "confirm_backtrack",
            Self::BacktrackRetry => "backtrack_retry",
            Self::BacktrackQuote => "backtrack_quote",
            Self::BacktrackFork => "backtrack_fork",
            Self::BacktrackCopy => "backtrack_copy",
            Self::BacktrackInspectTools => "backtrack_inspect_tools",
            Self::BacktrackDiff => "backtrack_diff",
            Self::HistoryPrev => "history_previous",
            Self::HistoryNext => "history_next",
            Self::SelectNextTool => "select_next_tool",
            Self::SelectPreviousTool => "select_previous_tool",
            Self::ToggleSelectedTool => "toggle_selected_tool",
            Self::CopySelectedTool => "copy_selected_tool",
            Self::CopyQuote => "copy_quote",
            Self::CopyOpenPath => "copy_open_path",
            Self::QueueDelete => "queue_delete",
            Self::QueueSteer => "queue_steer",
            Self::QueueRecall => "queue_recall",
            Self::TasksKill => "tasks_kill",
            Self::TasksRefresh => "tasks_refresh",
            Self::AskUserToggle => "ask_user_toggle",
            Self::AskUserDismiss => "ask_user_dismiss",
            Self::PermissionDismiss => "permission_dismiss",
            Self::SessionPin => "session_pin",
            Self::SessionArchive => "session_archive",
            Self::SessionDelete => "session_delete",
            Self::SessionRename => "session_rename",
            Self::SessionSlot1 => "session_slot_1",
            Self::SessionSlot2 => "session_slot_2",
            Self::SessionSlot3 => "session_slot_3",
            Self::SessionSlot4 => "session_slot_4",
            Self::SessionSlot5 => "session_slot_5",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        ALL_ACTIONS.iter().copied().find(|action| action.id() == id)
    }

    /// User-facing action description shared by help and diagnostics.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Quit => "Quit",
            Self::ConfirmQuit => "Confirm quit",
            Self::ClearInput => "Clear composer",
            Self::PasteRaw => "Paste next clipboard event without collapsing",
            Self::PasteImage => "Paste image from clipboard",
            Self::OpenHistorySearch => "Reverse search prompt history",
            Self::OpenTranscriptSearch => "Search current transcript",
            Self::OpenExternalEditor => "Edit draft in external editor",
            Self::Consumed | Self::Unhandled | Self::InputPassthrough => "Edit input",
            Self::Submit => "Submit or confirm",
            Self::CycleFocus => "Switch input / conversation focus",
            Self::ScrollUp => "Move or scroll up",
            Self::ScrollDown => "Move or scroll down",
            Self::PageScrollUp => "Move or scroll one page up",
            Self::PageScrollDown => "Move or scroll one page down",
            Self::ScrollToTop => "Scroll to top",
            Self::ScrollToBottom => "Scroll to bottom",
            Self::CancelStreaming => "Cancel the active response",
            Self::ShowHelp => "Show keyboard help",
            Self::ShowRawScrollback => "Show transcript in terminal scrollback",
            Self::EnterCommandMode => "Open command palette",
            Self::EnterShellMode => "Enter shell mode",
            Self::ReturnToNormal => "Close the current overlay",
            Self::EnterBacktrack => "Select an earlier prompt",
            Self::BacktrackPrevious => "Select the previous prompt",
            Self::BacktrackNext => "Select the next prompt",
            Self::ConfirmBacktrack => "Branch and edit the prompt",
            Self::BacktrackRetry => "Retry selected prompt in a branch",
            Self::BacktrackQuote => "Quote selected prompt into composer",
            Self::BacktrackFork => "Fork before selected prompt",
            Self::BacktrackCopy => "Copy selected prompt",
            Self::BacktrackInspectTools => "Inspect tools in selected turn",
            Self::BacktrackDiff => "Inspect changes in selected turn",
            Self::HistoryPrev => "Previous prompt history entry",
            Self::HistoryNext => "Next prompt history entry",
            Self::SelectNextTool => "Select next tool card",
            Self::SelectPreviousTool => "Select previous tool card",
            Self::ToggleSelectedTool => "Toggle tool details",
            Self::CopySelectedTool => "Copy selected tool",
            Self::CopyQuote => "Quote selected copy target",
            Self::CopyOpenPath => "Open selected path",
            Self::QueueDelete => "Delete queued follow-up",
            Self::QueueSteer => "Steer or un-steer follow-up",
            Self::QueueRecall => "Recall queued follow-up for editing",
            Self::TasksKill => "Kill selected background task",
            Self::TasksRefresh => "Refresh tasks",
            Self::AskUserToggle => "Toggle the focused answer",
            Self::AskUserDismiss => "Dismiss the question",
            Self::PermissionDismiss => "Deny or reject the prompt",
            Self::SessionPin => "Pin or unpin selected session",
            Self::SessionArchive => "Archive selected session",
            Self::SessionDelete => "Delete selected session with confirmation",
            Self::SessionRename => "Rename selected session",
            Self::SessionSlot1 => "Assign or open session slot 1",
            Self::SessionSlot2 => "Assign or open session slot 2",
            Self::SessionSlot3 => "Assign or open session slot 3",
            Self::SessionSlot4 => "Assign or open session slot 4",
            Self::SessionSlot5 => "Assign or open session slot 5",
        }
    }
}

const ALL_ACTIONS: &[KeyAction] = &[
    KeyAction::Quit,
    KeyAction::ConfirmQuit,
    KeyAction::ClearInput,
    KeyAction::PasteRaw,
    KeyAction::PasteImage,
    KeyAction::OpenHistorySearch,
    KeyAction::OpenTranscriptSearch,
    KeyAction::OpenExternalEditor,
    KeyAction::Submit,
    KeyAction::CycleFocus,
    KeyAction::ScrollUp,
    KeyAction::ScrollDown,
    KeyAction::PageScrollUp,
    KeyAction::PageScrollDown,
    KeyAction::ScrollToTop,
    KeyAction::ScrollToBottom,
    KeyAction::CancelStreaming,
    KeyAction::ShowHelp,
    KeyAction::ShowRawScrollback,
    KeyAction::EnterCommandMode,
    KeyAction::EnterShellMode,
    KeyAction::ReturnToNormal,
    KeyAction::EnterBacktrack,
    KeyAction::BacktrackPrevious,
    KeyAction::BacktrackNext,
    KeyAction::ConfirmBacktrack,
    KeyAction::BacktrackRetry,
    KeyAction::BacktrackQuote,
    KeyAction::BacktrackFork,
    KeyAction::BacktrackCopy,
    KeyAction::BacktrackInspectTools,
    KeyAction::BacktrackDiff,
    KeyAction::HistoryPrev,
    KeyAction::HistoryNext,
    KeyAction::SelectNextTool,
    KeyAction::SelectPreviousTool,
    KeyAction::ToggleSelectedTool,
    KeyAction::CopySelectedTool,
    KeyAction::CopyQuote,
    KeyAction::CopyOpenPath,
    KeyAction::QueueDelete,
    KeyAction::QueueSteer,
    KeyAction::QueueRecall,
    KeyAction::TasksKill,
    KeyAction::TasksRefresh,
    KeyAction::AskUserToggle,
    KeyAction::AskUserDismiss,
    KeyAction::PermissionDismiss,
    KeyAction::SessionPin,
    KeyAction::SessionArchive,
    KeyAction::SessionDelete,
    KeyAction::SessionRename,
    KeyAction::SessionSlot1,
    KeyAction::SessionSlot2,
    KeyAction::SessionSlot3,
    KeyAction::SessionSlot4,
    KeyAction::SessionSlot5,
];

/// Interaction context used to resolve the same chord differently by state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyContext {
    Global,
    Cancelling,
    Streaming,
    NormalInputEmpty,
    NormalInputDraft,
    NormalChat,
    Shell,
    Command,
    Select,
    Help,
    Picker,
    SessionHub,
    CopyPicker,
    Queue,
    Tasks,
    AskUser,
    Permission,
}

impl KeyContext {
    const fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Cancelling => "cancelling",
            Self::Streaming => "streaming",
            Self::NormalInputEmpty => "input (empty)",
            Self::NormalInputDraft => "input (draft)",
            Self::NormalChat => "conversation",
            Self::Shell => "shell",
            Self::Command => "command palette",
            Self::Select => "prompt backtrack",
            Self::Help => "help",
            Self::Picker => "picker",
            Self::SessionHub => "session hub",
            Self::CopyPicker => "copy picker",
            Self::Queue => "follow-up queue",
            Self::Tasks => "tasks",
            Self::AskUser => "AskUser prompt",
            Self::Permission => "permission prompt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct KeyChord {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyChord {
    fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        let mut chord = Self { code, modifiers };
        chord.normalize();
        chord
    }

    fn from_event(event: KeyEvent) -> Self {
        Self::new(event.code, event.modifiers)
    }

    fn parse(input: &str) -> Result<Self, KeymapError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(KeymapError::InvalidChord(input.into()));
        }
        let mut parts: Vec<&str> = input.split('+').collect();
        let key = parts.pop().unwrap_or_default();
        let mut modifiers = KeyModifiers::NONE;
        for modifier in parts {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers.insert(KeyModifiers::CONTROL),
                "alt" | "option" => modifiers.insert(KeyModifiers::ALT),
                "shift" => modifiers.insert(KeyModifiers::SHIFT),
                "super" | "cmd" | "meta" => modifiers.insert(KeyModifiers::SUPER),
                _ => return Err(KeymapError::InvalidChord(input.into())),
            }
        }
        let lower = key.to_ascii_lowercase();
        let code = match lower.as_str() {
            "enter" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backspace" => KeyCode::Backspace,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" => KeyCode::PageUp,
            "pagedown" => KeyCode::PageDown,
            _ if lower.starts_with('f') => lower[1..]
                .parse::<u8>()
                .ok()
                .filter(|number| (1..=24).contains(number))
                .map(KeyCode::F)
                .ok_or_else(|| KeymapError::InvalidChord(input.into()))?,
            _ => {
                let mut chars = key.chars();
                let character = chars
                    .next()
                    .filter(|_| chars.next().is_none())
                    .ok_or_else(|| KeymapError::InvalidChord(input.into()))?;
                KeyCode::Char(character)
            }
        };
        Ok(Self::new(code, modifiers))
    }

    fn normalize(&mut self) {
        self.modifiers &= KeyModifiers::SHIFT
            | KeyModifiers::CONTROL
            | KeyModifiers::ALT
            | KeyModifiers::SUPER
            | KeyModifiers::HYPER
            | KeyModifiers::META;
        if let KeyCode::Char(character) = self.code {
            if character.is_ascii_uppercase() {
                self.code = KeyCode::Char(character.to_ascii_lowercase());
                self.modifiers.insert(KeyModifiers::SHIFT);
            } else if !character.is_ascii_alphabetic() {
                self.modifiers.remove(KeyModifiers::SHIFT);
            }
        }
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            formatter.write_str("Ctrl+")?;
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            formatter.write_str("Alt+")?;
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            formatter.write_str("Shift+")?;
        }
        if self.modifiers.contains(KeyModifiers::SUPER) {
            formatter.write_str("Super+")?;
        }
        match self.code {
            KeyCode::Char(character)
                if character.is_ascii_alphabetic()
                    && self.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                    ) =>
            {
                write!(formatter, "{}", character.to_ascii_uppercase())
            }
            KeyCode::Char(character) => write!(formatter, "{character}"),
            KeyCode::Enter => formatter.write_str("Enter"),
            KeyCode::Esc => formatter.write_str("Esc"),
            KeyCode::Tab => formatter.write_str("Tab"),
            KeyCode::Backspace => formatter.write_str("Backspace"),
            KeyCode::Up => formatter.write_str("Up"),
            KeyCode::Down => formatter.write_str("Down"),
            KeyCode::Left => formatter.write_str("Left"),
            KeyCode::Right => formatter.write_str("Right"),
            KeyCode::Home => formatter.write_str("Home"),
            KeyCode::End => formatter.write_str("End"),
            KeyCode::PageUp => formatter.write_str("PageUp"),
            KeyCode::PageDown => formatter.write_str("PageDown"),
            KeyCode::F(number) => write!(formatter, "F{number}"),
            _ => write!(formatter, "{:?}", self.code),
        }
    }
}

#[derive(Debug, Clone)]
struct KeyBinding {
    context: KeyContext,
    chord: KeyChord,
    action: KeyAction,
}

/// One rendered help entry generated from the active keymap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHelpEntry {
    pub action: KeyAction,
    pub description: &'static str,
    pub keys: Vec<String>,
}

/// Keymap validation or parsing failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeymapError {
    UnknownAction(String),
    InvalidChord(String),
    Conflict {
        chord: String,
        context: String,
        first: String,
        second: String,
    },
    Read(String),
    Parse(String),
}

impl fmt::Display for KeymapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAction(action) => write!(formatter, "unknown keymap action `{action}`"),
            Self::InvalidChord(chord) => write!(formatter, "invalid key chord `{chord}`"),
            Self::Conflict {
                chord,
                context,
                first,
                second,
            } => write!(
                formatter,
                "key {chord} conflicts in {context}: `{first}` and `{second}`"
            ),
            Self::Read(error) => write!(formatter, "could not read keymap: {error}"),
            Self::Parse(error) => write!(formatter, "could not parse keymap: {error}"),
        }
    }
}

impl std::error::Error for KeymapError {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeymapFile {
    #[serde(default)]
    bindings: BTreeMap<String, Vec<String>>,
}

/// Validated semantic keymap used by dispatch, help, and diagnostics.
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: Vec<KeyBinding>,
}

impl Default for Keymap {
    fn default() -> Self {
        let keymap = Self {
            bindings: default_bindings(),
        };
        debug_assert!(keymap.validate().is_ok(), "built-in keymap must be valid");
        keymap
    }
}

impl Keymap {
    /// Load `[bindings]` overrides from a TOML file. A missing file uses defaults.
    pub fn load(path: &Path) -> Result<Self, KeymapError> {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(KeymapError::Read(error.to_string())),
        };
        let file: KeymapFile =
            toml::from_str(&source).map_err(|error| KeymapError::Parse(error.to_string()))?;
        Self::with_overrides(file.bindings)
    }

    pub fn load_for_terminal(
        path: &Path,
        capabilities: TerminalCapabilities,
    ) -> Result<Self, KeymapError> {
        let mut keymap = Self::load(path)?;
        keymap.add_terminal_fallbacks(capabilities);
        keymap.validate()?;
        Ok(keymap)
    }

    pub fn for_terminal(capabilities: TerminalCapabilities) -> Self {
        let mut keymap = Self::default();
        keymap.add_terminal_fallbacks(capabilities);
        keymap
    }

    fn add_terminal_fallbacks(&mut self, capabilities: TerminalCapabilities) {
        if !capabilities.needs_function_key_fallbacks() {
            return;
        }
        let fallback = KeyChord::new(KeyCode::F(8), KeyModifiers::NONE);
        for context in [
            KeyContext::NormalInputEmpty,
            KeyContext::NormalInputDraft,
            KeyContext::Shell,
        ] {
            let occupied = self
                .bindings
                .iter()
                .any(|binding| binding.context == context && binding.chord == fallback);
            if !occupied {
                self.bindings.push(KeyBinding {
                    context,
                    chord: fallback.clone(),
                    action: KeyAction::PasteRaw,
                });
            }
        }
    }

    /// Apply action-level overrides while preserving each action's contexts.
    pub fn with_overrides(overrides: BTreeMap<String, Vec<String>>) -> Result<Self, KeymapError> {
        let mut bindings = default_bindings();
        for (action_id, configured_chords) in overrides {
            let action = KeyAction::from_id(&action_id)
                .ok_or_else(|| KeymapError::UnknownAction(action_id.clone()))?;
            let contexts: Vec<KeyContext> = bindings
                .iter()
                .filter(|binding| binding.action == action)
                .map(|binding| binding.context)
                .collect();
            if contexts.is_empty() {
                return Err(KeymapError::UnknownAction(action_id));
            }
            let chords = configured_chords
                .iter()
                .map(|chord| KeyChord::parse(chord))
                .collect::<Result<Vec<_>, _>>()?;
            bindings.retain(|binding| binding.action != action);
            for context in contexts {
                for chord in &chords {
                    bindings.push(KeyBinding {
                        context,
                        chord: chord.clone(),
                        action,
                    });
                }
            }
        }
        let keymap = Self { bindings };
        keymap.validate()?;
        Ok(keymap)
    }

    /// Resolve an event using an empty composer for compatibility with tests.
    #[cfg(test)]
    pub fn dispatch(&self, key: KeyEvent, state: &AppState) -> KeyAction {
        self.dispatch_with_composer(key, state, true)
    }

    /// Resolve an event against active contexts and composer state.
    pub fn dispatch_with_composer(
        &self,
        key: KeyEvent,
        state: &AppState,
        composer_empty: bool,
    ) -> KeyAction {
        if key.kind != KeyEventKind::Press {
            return KeyAction::Consumed;
        }
        let chord = KeyChord::from_event(key);
        for context in active_contexts(state, composer_empty) {
            if let Some(binding) = self
                .bindings
                .iter()
                .find(|binding| binding.context == context && binding.chord == chord)
            {
                return binding.action;
            }
        }
        fallback_action(state)
    }

    /// Validate that no simultaneously active contexts claim the same chord.
    pub fn validate(&self) -> Result<(), KeymapError> {
        let mut seen: HashMap<&KeyChord, &KeyBinding> = HashMap::new();
        for binding in &self.bindings {
            if let Some(previous) = seen.get(&binding.chord) {
                if previous.action == binding.action {
                    continue;
                }
                if previous.context == binding.context
                    || previous.context == KeyContext::Global
                    || binding.context == KeyContext::Global
                {
                    let context = if previous.context == KeyContext::Global
                        || binding.context == KeyContext::Global
                    {
                        KeyContext::Global
                    } else {
                        binding.context
                    };
                    return Err(KeymapError::Conflict {
                        chord: binding.chord.to_string(),
                        context: context.label().into(),
                        first: previous.action.id().into(),
                        second: binding.action.id().into(),
                    });
                }
            } else {
                seen.insert(&binding.chord, binding);
            }
        }
        Ok(())
    }

    /// Group active bindings by action for generated help and footer hints.
    pub fn help_entries(&self, context: KeyContext) -> Vec<KeyHelpEntry> {
        let mut entries = Vec::new();
        for action in ALL_ACTIONS {
            let keys: Vec<String> = self
                .bindings
                .iter()
                .filter(|binding| binding.context == context && binding.action == *action)
                .map(|binding| binding.chord.to_string())
                .collect();
            if !keys.is_empty() {
                entries.push(KeyHelpEntry {
                    action: *action,
                    description: action.description(),
                    keys,
                });
            }
        }
        entries
    }
}

#[cfg(test)]
static DEFAULT_KEYMAP: LazyLock<Keymap> = LazyLock::new(Keymap::default);

/// Dispatch with the built-in keymap. Prefer [`Keymap::dispatch_with_composer`]
/// in the application so draft-sensitive actions resolve correctly.
#[cfg(test)]
pub fn dispatch(key: KeyEvent, state: &AppState) -> KeyAction {
    DEFAULT_KEYMAP.dispatch(key, state)
}

fn binding(context: KeyContext, chord: KeyChord, action: KeyAction) -> KeyBinding {
    KeyBinding {
        context,
        chord,
        action,
    }
}

fn plain(code: KeyCode) -> KeyChord {
    KeyChord::new(code, KeyModifiers::NONE)
}

fn character(character: char) -> KeyChord {
    plain(KeyCode::Char(character))
}

fn ctrl(character: char) -> KeyChord {
    KeyChord::new(KeyCode::Char(character), KeyModifiers::CONTROL)
}

fn shift(character: char) -> KeyChord {
    KeyChord::new(KeyCode::Char(character), KeyModifiers::SHIFT)
}

fn default_bindings() -> Vec<KeyBinding> {
    use KeyAction as A;
    use KeyContext as C;

    vec![
        binding(C::Global, ctrl('q'), A::Quit),
        binding(C::Global, plain(KeyCode::F(1)), A::ShowHelp),
        binding(C::Global, plain(KeyCode::F(3)), A::ShowRawScrollback),
        binding(C::Global, ctrl('o'), A::ToggleSelectedTool),
        binding(C::Cancelling, plain(KeyCode::Esc), A::Consumed),
        binding(C::Cancelling, ctrl('c'), A::Consumed),
        binding(C::Streaming, plain(KeyCode::Esc), A::CancelStreaming),
        binding(C::Streaming, ctrl('c'), A::CancelStreaming),
        binding(C::NormalInputEmpty, ctrl('c'), A::ConfirmQuit),
        binding(C::NormalInputEmpty, ctrl('d'), A::Quit),
        binding(C::NormalInputDraft, ctrl('c'), A::ClearInput),
        binding(C::NormalInputDraft, ctrl('d'), A::InputPassthrough),
        binding(
            C::NormalInputEmpty,
            KeyChord::new(KeyCode::Char('v'), KeyModifiers::ALT),
            A::PasteRaw,
        ),
        binding(
            C::NormalInputDraft,
            KeyChord::new(KeyCode::Char('v'), KeyModifiers::ALT),
            A::PasteRaw,
        ),
        binding(C::NormalInputEmpty, ctrl('v'), A::PasteImage),
        binding(C::NormalInputDraft, ctrl('v'), A::PasteImage),
        binding(C::NormalInputEmpty, ctrl('r'), A::OpenHistorySearch),
        binding(C::NormalInputDraft, ctrl('r'), A::OpenHistorySearch),
        binding(C::NormalInputEmpty, ctrl('f'), A::OpenTranscriptSearch),
        binding(C::NormalInputDraft, ctrl('f'), A::OpenTranscriptSearch),
        binding(C::NormalInputEmpty, ctrl('e'), A::OpenExternalEditor),
        binding(C::NormalInputDraft, ctrl('e'), A::OpenExternalEditor),
        binding(C::NormalInputEmpty, plain(KeyCode::Enter), A::Submit),
        binding(C::NormalInputDraft, plain(KeyCode::Enter), A::Submit),
        binding(
            C::NormalInputEmpty,
            KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT),
            A::InputPassthrough,
        ),
        binding(
            C::NormalInputDraft,
            KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT),
            A::InputPassthrough,
        ),
        // Alt+Enter inserts a newline on every ANSI terminal (ESC + CR) so the
        // composer can go multi-line even where Shift+Enter is indistinguishable
        // from Enter (the common case without the Kitty keyboard protocol).
        binding(
            C::NormalInputEmpty,
            KeyChord::new(KeyCode::Enter, KeyModifiers::ALT),
            A::InputPassthrough,
        ),
        binding(
            C::NormalInputDraft,
            KeyChord::new(KeyCode::Enter, KeyModifiers::ALT),
            A::InputPassthrough,
        ),
        binding(C::NormalInputEmpty, plain(KeyCode::Up), A::HistoryPrev),
        binding(C::NormalInputDraft, plain(KeyCode::Up), A::HistoryPrev),
        binding(C::NormalInputEmpty, plain(KeyCode::Down), A::HistoryNext),
        binding(C::NormalInputDraft, plain(KeyCode::Down), A::HistoryNext),
        binding(C::NormalInputEmpty, plain(KeyCode::Tab), A::CycleFocus),
        binding(C::NormalInputDraft, plain(KeyCode::Tab), A::CycleFocus),
        // Ctrl+G opens the draft in $VISUAL/$EDITOR (e.g. vim) so long-form
        // input can use the host editor. Mirrors other agent CLIs and keeps
        // Ctrl+E as an alias for the same action.
        binding(C::NormalInputEmpty, ctrl('g'), A::OpenExternalEditor),
        binding(C::NormalInputDraft, ctrl('g'), A::OpenExternalEditor),
        binding(C::NormalInputEmpty, character(':'), A::EnterCommandMode),
        binding(C::NormalInputDraft, character(':'), A::EnterCommandMode),
        binding(C::NormalInputEmpty, character('!'), A::EnterShellMode),
        binding(
            C::NormalInputEmpty,
            KeyChord::new(KeyCode::Char('1'), KeyModifiers::ALT),
            A::SessionSlot1,
        ),
        binding(
            C::NormalInputEmpty,
            KeyChord::new(KeyCode::Char('2'), KeyModifiers::ALT),
            A::SessionSlot2,
        ),
        binding(
            C::NormalInputEmpty,
            KeyChord::new(KeyCode::Char('3'), KeyModifiers::ALT),
            A::SessionSlot3,
        ),
        binding(
            C::NormalInputEmpty,
            KeyChord::new(KeyCode::Char('4'), KeyModifiers::ALT),
            A::SessionSlot4,
        ),
        binding(
            C::NormalInputEmpty,
            KeyChord::new(KeyCode::Char('5'), KeyModifiers::ALT),
            A::SessionSlot5,
        ),
        binding(
            C::NormalInputDraft,
            KeyChord::new(KeyCode::Char('1'), KeyModifiers::ALT),
            A::SessionSlot1,
        ),
        binding(
            C::NormalInputDraft,
            KeyChord::new(KeyCode::Char('2'), KeyModifiers::ALT),
            A::SessionSlot2,
        ),
        binding(
            C::NormalInputDraft,
            KeyChord::new(KeyCode::Char('3'), KeyModifiers::ALT),
            A::SessionSlot3,
        ),
        binding(
            C::NormalInputDraft,
            KeyChord::new(KeyCode::Char('4'), KeyModifiers::ALT),
            A::SessionSlot4,
        ),
        binding(
            C::NormalInputDraft,
            KeyChord::new(KeyCode::Char('5'), KeyModifiers::ALT),
            A::SessionSlot5,
        ),
        binding(C::NormalInputEmpty, plain(KeyCode::Esc), A::EnterBacktrack),
        binding(C::NormalInputDraft, plain(KeyCode::Esc), A::EnterBacktrack),
        binding(C::NormalChat, character(']'), A::SelectNextTool),
        binding(C::NormalChat, character('['), A::SelectPreviousTool),
        binding(C::NormalChat, plain(KeyCode::Enter), A::ToggleSelectedTool),
        binding(C::NormalChat, character('c'), A::CopySelectedTool),
        binding(C::NormalChat, plain(KeyCode::Up), A::ScrollUp),
        binding(C::NormalChat, character('k'), A::ScrollUp),
        binding(C::NormalChat, plain(KeyCode::Down), A::ScrollDown),
        binding(C::NormalChat, character('j'), A::ScrollDown),
        binding(C::NormalChat, plain(KeyCode::PageUp), A::PageScrollUp),
        binding(C::NormalChat, plain(KeyCode::PageDown), A::PageScrollDown),
        binding(C::NormalChat, plain(KeyCode::Home), A::ScrollToTop),
        binding(C::NormalChat, character('g'), A::ScrollToTop),
        binding(C::NormalChat, plain(KeyCode::End), A::ScrollToBottom),
        binding(C::NormalChat, shift('g'), A::ScrollToBottom),
        binding(C::NormalChat, plain(KeyCode::Tab), A::CycleFocus),
        binding(C::NormalChat, character('?'), A::ShowHelp),
        binding(C::NormalChat, ctrl('f'), A::OpenTranscriptSearch),
        binding(C::NormalChat, plain(KeyCode::Esc), A::EnterBacktrack),
        binding(C::NormalChat, character('i'), A::ReturnToNormal),
        binding(C::Shell, plain(KeyCode::Esc), A::ReturnToNormal),
        binding(C::Shell, ctrl('d'), A::ReturnToNormal),
        binding(C::Shell, ctrl('c'), A::ClearInput),
        binding(
            C::Shell,
            KeyChord::new(KeyCode::Char('v'), KeyModifiers::ALT),
            A::PasteRaw,
        ),
        binding(C::Shell, plain(KeyCode::Enter), A::Submit),
        binding(
            C::Shell,
            KeyChord::new(KeyCode::Enter, KeyModifiers::SHIFT),
            A::InputPassthrough,
        ),
        binding(
            C::Shell,
            KeyChord::new(KeyCode::Enter, KeyModifiers::ALT),
            A::InputPassthrough,
        ),
        binding(C::Shell, plain(KeyCode::Up), A::HistoryPrev),
        binding(C::Shell, plain(KeyCode::Down), A::HistoryNext),
        binding(C::Shell, ctrl('e'), A::OpenExternalEditor),
        binding(C::Command, plain(KeyCode::Esc), A::ReturnToNormal),
        binding(C::Command, plain(KeyCode::Enter), A::Submit),
        binding(C::Command, plain(KeyCode::Up), A::ScrollUp),
        binding(C::Command, plain(KeyCode::Down), A::ScrollDown),
        binding(C::Command, plain(KeyCode::Tab), A::ScrollDown),
        binding(C::Select, plain(KeyCode::Esc), A::ReturnToNormal),
        binding(C::Select, character('i'), A::ReturnToNormal),
        binding(C::Select, plain(KeyCode::Up), A::BacktrackPrevious),
        binding(C::Select, plain(KeyCode::Left), A::BacktrackPrevious),
        binding(C::Select, character('k'), A::BacktrackPrevious),
        binding(C::Select, plain(KeyCode::Down), A::BacktrackNext),
        binding(C::Select, plain(KeyCode::Right), A::BacktrackNext),
        binding(C::Select, character('j'), A::BacktrackNext),
        binding(C::Select, plain(KeyCode::Enter), A::ConfirmBacktrack),
        binding(C::Select, character('r'), A::BacktrackRetry),
        binding(C::Select, character('q'), A::BacktrackQuote),
        binding(C::Select, character('b'), A::BacktrackFork),
        binding(C::Select, character('y'), A::BacktrackCopy),
        binding(C::Select, character('t'), A::BacktrackInspectTools),
        binding(C::Select, character('d'), A::BacktrackDiff),
        binding(C::Help, plain(KeyCode::Esc), A::ReturnToNormal),
        binding(C::Help, character('q'), A::ReturnToNormal),
        binding(C::Help, plain(KeyCode::Up), A::ScrollUp),
        binding(C::Help, character('k'), A::ScrollUp),
        binding(C::Help, plain(KeyCode::Down), A::ScrollDown),
        binding(C::Help, character('j'), A::ScrollDown),
        binding(C::Help, plain(KeyCode::PageUp), A::PageScrollUp),
        binding(C::Help, plain(KeyCode::PageDown), A::PageScrollDown),
        binding(C::Picker, plain(KeyCode::Esc), A::ReturnToNormal),
        binding(C::Picker, plain(KeyCode::Enter), A::Submit),
        binding(C::Picker, plain(KeyCode::Up), A::ScrollUp),
        binding(C::Picker, plain(KeyCode::Down), A::ScrollDown),
        binding(C::Picker, plain(KeyCode::Tab), A::ScrollDown),
        binding(C::Picker, plain(KeyCode::PageUp), A::PageScrollUp),
        binding(C::Picker, plain(KeyCode::PageDown), A::PageScrollDown),
        binding(C::CopyPicker, plain(KeyCode::Esc), A::ReturnToNormal),
        binding(C::CopyPicker, plain(KeyCode::Enter), A::Submit),
        binding(C::CopyPicker, plain(KeyCode::Up), A::ScrollUp),
        binding(C::CopyPicker, plain(KeyCode::Down), A::ScrollDown),
        binding(C::CopyPicker, plain(KeyCode::PageUp), A::PageScrollUp),
        binding(C::CopyPicker, plain(KeyCode::PageDown), A::PageScrollDown),
        binding(
            C::CopyPicker,
            KeyChord::new(KeyCode::Enter, KeyModifiers::ALT),
            A::CopyQuote,
        ),
        binding(C::CopyPicker, ctrl('l'), A::CopyOpenPath),
        binding(C::SessionHub, plain(KeyCode::Esc), A::ReturnToNormal),
        binding(C::SessionHub, plain(KeyCode::Enter), A::Submit),
        binding(C::SessionHub, plain(KeyCode::Up), A::ScrollUp),
        binding(C::SessionHub, plain(KeyCode::Down), A::ScrollDown),
        binding(C::SessionHub, plain(KeyCode::PageUp), A::PageScrollUp),
        binding(C::SessionHub, plain(KeyCode::PageDown), A::PageScrollDown),
        binding(C::SessionHub, plain(KeyCode::F(2)), A::SessionRename),
        binding(C::SessionHub, ctrl('p'), A::SessionPin),
        binding(C::SessionHub, ctrl('a'), A::SessionArchive),
        binding(C::SessionHub, ctrl('d'), A::SessionDelete),
        binding(
            C::SessionHub,
            KeyChord::new(KeyCode::Char('1'), KeyModifiers::ALT),
            A::SessionSlot1,
        ),
        binding(
            C::SessionHub,
            KeyChord::new(KeyCode::Char('2'), KeyModifiers::ALT),
            A::SessionSlot2,
        ),
        binding(
            C::SessionHub,
            KeyChord::new(KeyCode::Char('3'), KeyModifiers::ALT),
            A::SessionSlot3,
        ),
        binding(
            C::SessionHub,
            KeyChord::new(KeyCode::Char('4'), KeyModifiers::ALT),
            A::SessionSlot4,
        ),
        binding(
            C::SessionHub,
            KeyChord::new(KeyCode::Char('5'), KeyModifiers::ALT),
            A::SessionSlot5,
        ),
        binding(C::Queue, plain(KeyCode::Esc), A::ReturnToNormal),
        binding(C::Queue, character('q'), A::ReturnToNormal),
        binding(C::Queue, plain(KeyCode::Enter), A::Submit),
        binding(C::Queue, plain(KeyCode::Up), A::ScrollUp),
        binding(C::Queue, character('k'), A::ScrollUp),
        binding(C::Queue, plain(KeyCode::Down), A::ScrollDown),
        binding(C::Queue, character('j'), A::ScrollDown),
        binding(C::Queue, character('d'), A::QueueDelete),
        binding(C::Queue, character('s'), A::QueueSteer),
        binding(C::Queue, character('e'), A::QueueRecall),
        binding(C::Tasks, plain(KeyCode::Esc), A::ReturnToNormal),
        binding(C::Tasks, character('q'), A::ReturnToNormal),
        binding(C::Tasks, plain(KeyCode::Enter), A::Submit),
        binding(C::Tasks, plain(KeyCode::Up), A::ScrollUp),
        binding(C::Tasks, character('k'), A::ScrollUp),
        binding(C::Tasks, plain(KeyCode::Down), A::ScrollDown),
        binding(C::Tasks, character('j'), A::ScrollDown),
        binding(C::Tasks, character('d'), A::TasksKill),
        binding(C::Tasks, character('r'), A::TasksRefresh),
        binding(C::AskUser, plain(KeyCode::Esc), A::AskUserDismiss),
        binding(C::AskUser, ctrl('c'), A::AskUserDismiss),
        binding(C::AskUser, plain(KeyCode::Enter), A::Submit),
        binding(C::AskUser, plain(KeyCode::Up), A::ScrollUp),
        binding(C::AskUser, plain(KeyCode::Down), A::ScrollDown),
        binding(C::AskUser, plain(KeyCode::Char(' ')), A::AskUserToggle),
        binding(C::Permission, plain(KeyCode::Esc), A::PermissionDismiss),
        binding(C::Permission, ctrl('c'), A::PermissionDismiss),
        binding(C::Permission, plain(KeyCode::Enter), A::Submit),
        binding(C::Permission, plain(KeyCode::Up), A::ScrollUp),
        binding(C::Permission, plain(KeyCode::Down), A::ScrollDown),
    ]
}

fn active_contexts(state: &AppState, composer_empty: bool) -> Vec<KeyContext> {
    let mut contexts = vec![KeyContext::Global];
    if state.is_cancelling {
        contexts.insert(0, KeyContext::Cancelling);
    } else if state.is_streaming
        && state.mode != InteractionMode::AskUser
        && state.mode != InteractionMode::Permission
        && state.mode != InteractionMode::PlanReview
    {
        contexts.insert(0, KeyContext::Streaming);
    }
    let mode = match state.mode {
        InteractionMode::Normal if state.focus == PanelFocus::Input && composer_empty => {
            KeyContext::NormalInputEmpty
        }
        InteractionMode::Normal if state.focus == PanelFocus::Input => KeyContext::NormalInputDraft,
        InteractionMode::Normal => KeyContext::NormalChat,
        InteractionMode::Shell => KeyContext::Shell,
        InteractionMode::Command => KeyContext::Command,
        InteractionMode::Select => KeyContext::Select,
        InteractionMode::Help => KeyContext::Help,
        InteractionMode::Queue => KeyContext::Queue,
        InteractionMode::Tasks => KeyContext::Tasks,
        InteractionMode::AskUser => KeyContext::AskUser,
        InteractionMode::Permission | InteractionMode::PlanReview => KeyContext::Permission,
        InteractionMode::Resume => KeyContext::SessionHub,
        InteractionMode::Copy => KeyContext::CopyPicker,
        InteractionMode::HistorySearch
        | InteractionMode::TranscriptSearch
        | InteractionMode::Prompt => KeyContext::Picker,
    };
    contexts.push(mode);
    contexts
}

fn fallback_action(state: &AppState) -> KeyAction {
    match state.mode {
        InteractionMode::Normal if state.focus == PanelFocus::Input => KeyAction::InputPassthrough,
        InteractionMode::Shell
        | InteractionMode::Command
        | InteractionMode::Copy
        | InteractionMode::HistorySearch
        | InteractionMode::TranscriptSearch
        | InteractionMode::Resume
        | InteractionMode::Prompt
        | InteractionMode::AskUser => KeyAction::InputPassthrough,
        InteractionMode::Help
        | InteractionMode::Queue
        | InteractionMode::Tasks
        | InteractionMode::Permission
        | InteractionMode::PlanReview => KeyAction::Consumed,
        InteractionMode::Normal | InteractionMode::Select => KeyAction::Unhandled,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_with_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // T-TUI-03-01: Ctrl+Q always quits regardless of mode.
    #[test]
    fn test_ctrl_q_quits_any_mode() {
        let modes = [
            InteractionMode::Normal,
            InteractionMode::Command,
            InteractionMode::Select,
        ];
        for mode in &modes {
            let mut state = AppState::new();
            state.mode = *mode;
            let action = dispatch(
                key_with_mod(KeyCode::Char('q'), KeyModifiers::CONTROL),
                &state,
            );
            assert_eq!(action, KeyAction::Quit, "Ctrl+Q should quit in {mode:?}");
        }
    }

    // T-TUI-03-02: Tab cycles focus in normal mode.
    #[test]
    fn test_tab_cycles_focus() {
        let state = AppState::new(); // focus = Input, mode = Normal
        let action = dispatch(key(KeyCode::Tab), &state);
        assert_eq!(action, KeyAction::CycleFocus);
    }

    // T-TUI-03-03: Enter submits in input-focused normal mode.
    #[test]
    fn test_enter_submits_in_input_normal() {
        let state = AppState::new();
        let action = dispatch(key(KeyCode::Enter), &state);
        assert_eq!(action, KeyAction::Submit);
    }

    // T-TUI-03-04: Shift+Enter passes through as newline in input.
    #[test]
    fn test_shift_enter_passthrough() {
        let state = AppState::new();
        let action = dispatch(key_with_mod(KeyCode::Enter, KeyModifiers::SHIFT), &state);
        assert_eq!(action, KeyAction::InputPassthrough);
    }

    #[test]
    fn test_composer_productivity_shortcuts_are_semantic_actions() {
        let state = AppState::new();
        assert_eq!(
            dispatch(
                key_with_mod(KeyCode::Char('r'), KeyModifiers::CONTROL),
                &state,
            ),
            KeyAction::OpenHistorySearch
        );
        assert_eq!(
            dispatch(
                key_with_mod(KeyCode::Char('e'), KeyModifiers::CONTROL),
                &state,
            ),
            KeyAction::OpenExternalEditor
        );
    }

    #[test]
    fn test_queue_edit_recalls_selected_prompt() {
        let mut state = AppState::new();
        state.mode = InteractionMode::Queue;
        assert_eq!(
            dispatch(key(KeyCode::Char('e')), &state),
            KeyAction::QueueRecall
        );
    }

    #[test]
    fn test_remote_terminal_adds_raw_paste_function_key_fallback() {
        let keymap = Keymap::for_terminal(TerminalCapabilities::for_host(
            crate::tui::terminal::TerminalHost::Ssh,
        ));
        let state = AppState::new();

        assert_eq!(
            keymap.dispatch(key(KeyCode::F(8)), &state),
            KeyAction::PasteRaw
        );
    }

    #[test]
    fn test_escape_is_consumed_when_cancellation_is_already_pending() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.is_cancelling = true;

        let action = dispatch(key(KeyCode::Esc), &state);

        assert_eq!(action, KeyAction::Consumed);
    }

    #[test]
    fn test_idle_escape_enters_backtrack_selection() {
        let state = AppState::new();

        assert_eq!(
            dispatch(key(KeyCode::Esc), &state),
            KeyAction::EnterBacktrack
        );
    }

    // T-TUI-03-05: j/k scroll in chat-focused normal mode.
    #[test]
    fn test_jk_scroll_chat() {
        let mut state = AppState::new();
        state.focus = PanelFocus::Chat;

        assert_eq!(
            dispatch(key(KeyCode::Char('j')), &state),
            KeyAction::ScrollDown
        );
        assert_eq!(
            dispatch(key(KeyCode::Char('k')), &state),
            KeyAction::ScrollUp
        );
    }

    // Ctrl+O toggles the selected tool card from any mode and focus, so the
    // latest card can be expanded without leaving the composer.
    #[test]
    fn test_ctrl_o_toggles_tool_any_mode() {
        let modes = [
            InteractionMode::Normal,
            InteractionMode::Command,
            InteractionMode::Select,
            InteractionMode::Queue,
        ];
        for mode in &modes {
            let mut state = AppState::new();
            state.mode = *mode;
            let action = dispatch(
                key_with_mod(KeyCode::Char('o'), KeyModifiers::CONTROL),
                &state,
            );
            assert_eq!(
                action,
                KeyAction::ToggleSelectedTool,
                "Ctrl+O should toggle the tool card in {mode:?}"
            );
        }
    }

    // A bare 'o' (no Ctrl) must keep its per-mode meaning, e.g. composer text.
    #[test]
    fn test_plain_o_is_not_a_tool_toggle() {
        let state = AppState::new();
        let action = dispatch(key(KeyCode::Char('o')), &state);
        assert_eq!(action, KeyAction::InputPassthrough);
    }

    // T-TUI-03-06: Ctrl+B is no longer reserved after removing the sidebar.
    #[test]
    fn test_ctrl_b_is_available_to_input_editor() {
        let state = AppState::new();
        let action = dispatch(
            key_with_mod(KeyCode::Char('b'), KeyModifiers::CONTROL),
            &state,
        );
        assert_eq!(action, KeyAction::InputPassthrough);
    }

    // F1 shows help regardless of mode (replaces the old Ctrl+H binding,
    // which collided with Backspace on terminals that send ^H for it).
    #[test]
    fn test_f1_shows_help_any_mode() {
        let modes = [
            InteractionMode::Normal,
            InteractionMode::Command,
            InteractionMode::Select,
        ];
        for mode in &modes {
            let mut state = AppState::new();
            state.mode = *mode;
            let action = dispatch(key(KeyCode::F(1)), &state);
            assert_eq!(
                action,
                KeyAction::ShowHelp,
                "F1 should show help in {mode:?}"
            );
        }
    }

    // Ctrl+H must reach the input editor: many terminals report Backspace
    // as ^H, so it must not trigger the help overlay.
    #[test]
    fn test_ctrl_h_reaches_input_editor() {
        let state = AppState::new();
        let action = dispatch(
            key_with_mod(KeyCode::Char('h'), KeyModifiers::CONTROL),
            &state,
        );
        assert_eq!(action, KeyAction::InputPassthrough);
    }

    // Release/Repeat events must not dispatch actions, otherwise platforms
    // that emit them (e.g. Windows) would handle one keystroke twice.
    #[test]
    fn test_non_press_key_events_are_consumed() {
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let state = AppState::new();
            let quit = KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::CONTROL,
                kind,
                state: KeyEventState::NONE,
            };
            assert_eq!(
                dispatch(quit, &state),
                KeyAction::Consumed,
                "{kind:?} events must not trigger actions"
            );

            let submit = KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                kind,
                state: KeyEventState::NONE,
            };
            assert_eq!(
                dispatch(submit, &state),
                KeyAction::Consumed,
                "{kind:?} events must not trigger actions"
            );
        }
    }

    // T-TUI-03-07: Escape returns to normal from command mode.
    #[test]
    fn test_escape_command_to_normal() {
        let mut state = AppState::new();
        state.mode = InteractionMode::Command;
        let action = dispatch(key(KeyCode::Esc), &state);
        assert_eq!(action, KeyAction::ReturnToNormal);
    }

    // T-TUI-03-08: Colon enters command mode from input.
    #[test]
    fn test_colon_enters_command_mode() {
        let state = AppState::new(); // Input focus, Normal mode
        let action = dispatch(key(KeyCode::Char(':')), &state);
        assert_eq!(action, KeyAction::EnterCommandMode);
    }

    #[test]
    fn test_bang_enters_shell_mode_only_from_empty_composer() {
        let state = AppState::new();

        assert_eq!(
            DEFAULT_KEYMAP.dispatch_with_composer(key(KeyCode::Char('!')), &state, true),
            KeyAction::EnterShellMode
        );
        assert_eq!(
            DEFAULT_KEYMAP.dispatch_with_composer(key(KeyCode::Char('!')), &state, false),
            KeyAction::InputPassthrough
        );
    }

    #[test]
    fn test_shell_mode_submits_and_escape_exits() {
        let mut state = AppState::new();
        state.mode = InteractionMode::Shell;

        assert_eq!(dispatch(key(KeyCode::Enter), &state), KeyAction::Submit);
        assert_eq!(
            dispatch(
                key_with_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &state
            ),
            KeyAction::ClearInput
        );
        assert_eq!(
            dispatch(key(KeyCode::Esc), &state),
            KeyAction::ReturnToNormal
        );
        assert_eq!(
            dispatch(key(KeyCode::Char('a')), &state),
            KeyAction::InputPassthrough
        );
    }

    // T-TUI-03-09: Regular chars pass through to textarea in input.
    #[test]
    fn test_char_passthrough_input() {
        let state = AppState::new();
        let action = dispatch(key(KeyCode::Char('a')), &state);
        assert_eq!(action, KeyAction::InputPassthrough);
    }

    // T-TUI-03-11: Select mode scrolls with arrows; Esc exits the mode.
    #[test]
    fn test_select_mode_scroll() {
        let mut state = AppState::new();
        state.mode = InteractionMode::Select;
        assert_eq!(
            dispatch(key(KeyCode::Esc), &state),
            KeyAction::ReturnToNormal
        );
        assert_eq!(
            dispatch(key(KeyCode::Up), &state),
            KeyAction::BacktrackPrevious
        );
        assert_eq!(
            dispatch(key(KeyCode::Down), &state),
            KeyAction::BacktrackNext
        );
        assert_eq!(
            dispatch(key(KeyCode::Enter), &state),
            KeyAction::ConfirmBacktrack
        );
        assert_eq!(
            dispatch(key(KeyCode::Char('q')), &state),
            KeyAction::BacktrackQuote
        );
    }

    #[test]
    fn test_chat_focus_exposes_tool_navigation_actions() {
        let mut state = AppState::new();
        state.focus = PanelFocus::Chat;

        assert_eq!(
            dispatch(key(KeyCode::Char(']')), &state),
            KeyAction::SelectNextTool
        );
        assert_eq!(
            dispatch(key(KeyCode::Char('[')), &state),
            KeyAction::SelectPreviousTool
        );
        assert_eq!(
            dispatch(key(KeyCode::Enter), &state),
            KeyAction::ToggleSelectedTool
        );
        assert_eq!(
            dispatch(key(KeyCode::Char('c')), &state),
            KeyAction::CopySelectedTool
        );
    }

    #[test]
    fn test_prompt_picker_uses_shared_picker_keys() {
        let mut state = AppState::new();
        state.mode = InteractionMode::Prompt;

        assert_eq!(dispatch(key(KeyCode::Enter), &state), KeyAction::Submit);
        assert_eq!(dispatch(key(KeyCode::Up), &state), KeyAction::ScrollUp);
        assert_eq!(
            dispatch(key(KeyCode::Esc), &state),
            KeyAction::ReturnToNormal
        );
    }

    #[test]
    fn test_queue_mode_dispatch_table() {
        let mut state = AppState::new();
        state.mode = InteractionMode::Queue;

        // Close keys.
        assert_eq!(
            dispatch(key(KeyCode::Esc), &state),
            KeyAction::ReturnToNormal
        );
        assert_eq!(
            dispatch(key(KeyCode::Char('q')), &state),
            KeyAction::ReturnToNormal
        );
        assert_eq!(dispatch(key(KeyCode::Enter), &state), KeyAction::Submit);

        // Navigation keys.
        assert_eq!(dispatch(key(KeyCode::Up), &state), KeyAction::ScrollUp);
        assert_eq!(
            dispatch(key(KeyCode::Char('k')), &state),
            KeyAction::ScrollUp
        );
        assert_eq!(dispatch(key(KeyCode::Down), &state), KeyAction::ScrollDown);
        assert_eq!(
            dispatch(key(KeyCode::Char('j')), &state),
            KeyAction::ScrollDown
        );

        // Queue actions.
        assert_eq!(
            dispatch(key(KeyCode::Char('d')), &state),
            KeyAction::QueueDelete
        );
        assert_eq!(
            dispatch(key(KeyCode::Char('s')), &state),
            KeyAction::QueueSteer
        );

        // Anything else is consumed, never passed through to the composer.
        assert_eq!(
            dispatch(key(KeyCode::Char('x')), &state),
            KeyAction::Consumed
        );
        assert_eq!(
            dispatch(key(KeyCode::Backspace), &state),
            KeyAction::Consumed
        );
    }

    #[test]
    fn test_tasks_mode_dispatch_table() {
        let mut state = AppState::new();
        state.mode = InteractionMode::Tasks;

        // Close keys.
        assert_eq!(
            dispatch(key(KeyCode::Esc), &state),
            KeyAction::ReturnToNormal
        );
        assert_eq!(
            dispatch(key(KeyCode::Char('q')), &state),
            KeyAction::ReturnToNormal
        );
        assert_eq!(dispatch(key(KeyCode::Enter), &state), KeyAction::Submit);

        // Navigation keys, consistent with the queue overlay (k scrolls up).
        assert_eq!(dispatch(key(KeyCode::Up), &state), KeyAction::ScrollUp);
        assert_eq!(
            dispatch(key(KeyCode::Char('k')), &state),
            KeyAction::ScrollUp
        );
        assert_eq!(dispatch(key(KeyCode::Down), &state), KeyAction::ScrollDown);
        assert_eq!(
            dispatch(key(KeyCode::Char('j')), &state),
            KeyAction::ScrollDown
        );

        // Task actions: kill sits on `d` like the queue overlay's delete.
        assert_eq!(
            dispatch(key(KeyCode::Char('d')), &state),
            KeyAction::TasksKill
        );
        assert_eq!(
            dispatch(key(KeyCode::Char('r')), &state),
            KeyAction::TasksRefresh
        );

        // Anything else is consumed, never passed through to the composer.
        assert_eq!(
            dispatch(key(KeyCode::Char('x')), &state),
            KeyAction::Consumed
        );
        assert_eq!(
            dispatch(key(KeyCode::Backspace), &state),
            KeyAction::Consumed
        );
    }

    #[test]
    fn test_default_keymap_has_no_context_conflicts() {
        let keymap = Keymap::default();

        assert!(keymap.validate().is_ok());
    }

    #[test]
    fn test_keymap_metadata_is_the_help_source_of_truth() {
        let keymap = Keymap::default();
        let help = keymap.help_entries(KeyContext::Global);

        assert!(help.iter().any(|entry| {
            entry.action == KeyAction::ShowHelp && entry.keys.iter().any(|key| key == "F1")
        }));
        assert!(!help
            .iter()
            .any(|entry| entry.keys.iter().any(|key| key == "Ctrl+H")));
    }

    #[test]
    fn test_keymap_override_replaces_default_binding() {
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("show_help".to_string(), vec!["ctrl+?".to_string()]);
        let keymap = Keymap::with_overrides(overrides).unwrap();
        let state = AppState::new();

        assert_eq!(
            keymap.dispatch(
                key_with_mod(KeyCode::Char('?'), KeyModifiers::CONTROL),
                &state,
            ),
            KeyAction::ShowHelp
        );
        assert_ne!(
            keymap.dispatch(key(KeyCode::F(1)), &state),
            KeyAction::ShowHelp
        );
    }

    #[test]
    fn test_keymap_rejects_unknown_action_with_action_name() {
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("teleport".to_string(), vec!["ctrl+t".to_string()]);

        let error = Keymap::with_overrides(overrides).unwrap_err();

        assert!(error.to_string().contains("teleport"));
    }

    #[test]
    fn test_keymap_rejects_conflicting_binding_with_context() {
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("quit".to_string(), vec!["f1".to_string()]);

        let error = Keymap::with_overrides(overrides).unwrap_err();

        assert!(error.to_string().contains("F1"));
        assert!(error.to_string().contains("global"));
    }

    #[test]
    fn test_ctrl_c_is_state_sensitive() {
        let keymap = Keymap::default();
        let ctrl_c = key_with_mod(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut state = AppState::new();

        assert_eq!(
            keymap.dispatch_with_composer(ctrl_c, &state, true),
            KeyAction::ConfirmQuit
        );
        assert_eq!(
            keymap.dispatch_with_composer(ctrl_c, &state, false),
            KeyAction::ClearInput
        );

        state.is_streaming = true;
        assert_eq!(
            keymap.dispatch_with_composer(ctrl_c, &state, false),
            KeyAction::CancelStreaming
        );
    }

    #[test]
    fn test_ctrl_d_only_quits_with_empty_composer() {
        let keymap = Keymap::default();
        let ctrl_d = key_with_mod(KeyCode::Char('d'), KeyModifiers::CONTROL);
        let state = AppState::new();

        assert_eq!(
            keymap.dispatch_with_composer(ctrl_d, &state, true),
            KeyAction::Quit
        );
        assert_eq!(
            keymap.dispatch_with_composer(ctrl_d, &state, false),
            KeyAction::InputPassthrough
        );
    }

    #[test]
    fn test_alt_v_arms_raw_paste_in_composer() {
        let keymap = Keymap::default();
        let state = AppState::new();

        assert_eq!(
            keymap.dispatch_with_composer(
                key_with_mod(KeyCode::Char('v'), KeyModifiers::ALT),
                &state,
                false,
            ),
            KeyAction::PasteRaw
        );
    }

    #[test]
    fn test_ctrl_v_requests_clipboard_image_in_composer() {
        let keymap = Keymap::default();
        let state = AppState::new();

        assert_eq!(
            keymap.dispatch_with_composer(
                key_with_mod(KeyCode::Char('v'), KeyModifiers::CONTROL),
                &state,
                true,
            ),
            KeyAction::PasteImage
        );
    }

    #[test]
    fn test_ask_user_bindings_override_stream_cancellation() {
        let keymap = Keymap::default();
        let mut state = AppState::new();
        state.mode = InteractionMode::AskUser;
        state.is_streaming = true;

        assert_eq!(
            keymap.dispatch_with_composer(key(KeyCode::Esc), &state, true),
            KeyAction::AskUserDismiss
        );
        assert_eq!(dispatch(key(KeyCode::Enter), &state), KeyAction::Submit);
        assert_eq!(
            dispatch(key(KeyCode::Char(' ')), &state),
            KeyAction::AskUserToggle
        );
        assert_eq!(dispatch(key(KeyCode::Up), &state), KeyAction::ScrollUp);
        assert_eq!(
            dispatch(key(KeyCode::Char('x')), &state),
            KeyAction::InputPassthrough
        );
    }

    #[test]
    fn test_permission_bindings_override_stream_cancellation() {
        let keymap = Keymap::default();
        let mut state = AppState::new();
        state.is_streaming = true;

        for mode in [InteractionMode::Permission, InteractionMode::PlanReview] {
            state.mode = mode;
            assert_eq!(
                keymap.dispatch_with_composer(key(KeyCode::Esc), &state, true),
                KeyAction::PermissionDismiss,
                "Esc must answer the prompt, not cancel the run ({mode:?})"
            );
            assert_eq!(dispatch(key(KeyCode::Enter), &state), KeyAction::Submit);
            assert_eq!(dispatch(key(KeyCode::Up), &state), KeyAction::ScrollUp);
            assert_eq!(dispatch(key(KeyCode::Down), &state), KeyAction::ScrollDown);
            assert_eq!(
                dispatch(key(KeyCode::Char('x')), &state),
                KeyAction::Consumed,
                "prompts have no text target ({mode:?})"
            );
        }
    }
}
