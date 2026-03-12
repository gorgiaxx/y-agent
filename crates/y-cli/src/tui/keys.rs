//! Key dispatcher: maps key events to state transitions based on mode and focus.
//!
//! The dispatcher follows a two-tier priority:
//! 1. **Global keys** (Ctrl+Q, Ctrl+D, Ctrl+C) — always handled, regardless of mode/focus.
//! 2. **Mode + Focus keys** — dispatched based on `InteractionMode` × `PanelFocus`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::state::{AppState, InteractionMode, PanelFocus};

/// Result of dispatching a key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Quit the TUI application.
    Quit,
    /// No-op — the key was consumed but had no effect.
    Consumed,
    /// The key was not handled by the dispatcher.
    Unhandled,
    /// Submit the current input buffer.
    Submit,
    /// A character/edit to pass through to the textarea.
    InputPassthrough,
    /// Toggle sidebar visibility.
    ToggleSidebar,
    /// Switch sidebar tab view.
    ToggleSidebarView,
    /// Cycle focus forward.
    CycleFocus,
    /// Scroll chat up.
    ScrollUp,
    /// Scroll chat down.
    ScrollDown,
    /// Enter command mode.
    EnterCommandMode,
    /// Return to normal mode.
    ReturnToNormal,
    /// Navigate to previous input history entry.
    HistoryPrev,
    /// Navigate to next input history entry.
    HistoryNext,
    /// Select the highlighted session item in the sidebar.
    SelectSessionItem,
}

/// Dispatch a key event against the current state.
///
/// Returns a `KeyAction` indicating what the caller should do.
pub fn dispatch(key: KeyEvent, state: &AppState) -> KeyAction {
    // Tier 1: Global shortcuts (always active).
    if let Some(action) = dispatch_global(key) {
        return action;
    }

    // Tier 2: Mode-specific dispatch.
    match state.mode {
        InteractionMode::Normal => dispatch_normal(key, state),
        InteractionMode::Command => dispatch_command(key, state),
        InteractionMode::Search => dispatch_search(key),
        InteractionMode::Select => dispatch_select(key),
    }
}

// ---------------------------------------------------------------------------
// Tier 1: Global keys
// ---------------------------------------------------------------------------

fn dispatch_global(key: KeyEvent) -> Option<KeyAction> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char('q' | 'd' | 'c') = key.code {
            return Some(KeyAction::Quit);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tier 2: Normal mode
// ---------------------------------------------------------------------------

fn dispatch_normal(key: KeyEvent, state: &AppState) -> KeyAction {
    match state.focus {
        PanelFocus::Input => dispatch_input_normal(key),
        PanelFocus::Chat => dispatch_chat_normal(key),
        PanelFocus::Sidebar => dispatch_sidebar_normal(key),
    }
}

/// Normal mode, Input panel focused.
fn dispatch_input_normal(key: KeyEvent) -> KeyAction {
    match key.code {
        // Enter submits the message.
        KeyCode::Enter => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                KeyAction::InputPassthrough // Shift+Enter = newline
            } else {
                KeyAction::Submit
            }
        }
        // Up/Down navigate input history.
        KeyCode::Up => KeyAction::HistoryPrev,
        KeyCode::Down => KeyAction::HistoryNext,
        // Tab cycles focus.
        KeyCode::Tab => KeyAction::CycleFocus,
        // Ctrl+B toggles sidebar.
        _ if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('b') => {
            KeyAction::ToggleSidebar
        }
        // ':' prefix enters command mode.
        KeyCode::Char(':') => KeyAction::EnterCommandMode,
        // Escape returns to normal (no-op in normal, but clears any partial).
        KeyCode::Esc => KeyAction::ReturnToNormal,
        // Everything else passes through to textarea.
        _ => KeyAction::InputPassthrough,
    }
}

/// Normal mode, Chat panel focused.
fn dispatch_chat_normal(key: KeyEvent) -> KeyAction {
    match key.code {
        // Scroll navigation.
        KeyCode::Up | KeyCode::Char('k') => KeyAction::ScrollUp,
        KeyCode::Down | KeyCode::Char('j') => KeyAction::ScrollDown,
        KeyCode::PageUp => KeyAction::ScrollUp,
        KeyCode::PageDown => KeyAction::ScrollDown,
        // Tab cycles focus.
        KeyCode::Tab => KeyAction::CycleFocus,
        // Ctrl+B toggles sidebar.
        _ if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('b') => {
            KeyAction::ToggleSidebar
        }
        // Escape or 'i' returns focus to input.
        KeyCode::Esc | KeyCode::Char('i') => KeyAction::ReturnToNormal,
        _ => KeyAction::Unhandled,
    }
}

/// Normal mode, Sidebar panel focused.
fn dispatch_sidebar_normal(key: KeyEvent) -> KeyAction {
    match key.code {
        // Tab views within sidebar.
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => KeyAction::ToggleSidebarView,
        // Tab cycles focus.
        KeyCode::Tab => KeyAction::CycleFocus,
        // Ctrl+B toggles sidebar.
        _ if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('b') => {
            KeyAction::ToggleSidebar
        }
        // Up/Down navigate session list.
        KeyCode::Up | KeyCode::Char('k') => KeyAction::ScrollUp,
        KeyCode::Down | KeyCode::Char('j') => KeyAction::ScrollDown,
        // Enter selects the highlighted session.
        KeyCode::Enter => KeyAction::SelectSessionItem,
        // Escape returns focus to input.
        KeyCode::Esc => KeyAction::ReturnToNormal,
        _ => KeyAction::Unhandled,
    }
}

// ---------------------------------------------------------------------------
// Tier 2: Command mode
// ---------------------------------------------------------------------------

fn dispatch_command(key: KeyEvent, _state: &AppState) -> KeyAction {
    match key.code {
        // Escape cancels command mode, returns to normal.
        KeyCode::Esc => KeyAction::ReturnToNormal,
        // Enter submits the command.
        KeyCode::Enter => KeyAction::Submit,
        // Arrow keys navigate the palette selection.
        KeyCode::Up => KeyAction::ScrollUp,
        KeyCode::Down => KeyAction::ScrollDown,
        // Tab can also cycle through results.
        KeyCode::Tab => KeyAction::ScrollDown,
        // Everything else is input for the command buffer.
        _ => KeyAction::InputPassthrough,
    }
}

// ---------------------------------------------------------------------------
// Tier 2: Search mode
// ---------------------------------------------------------------------------

fn dispatch_search(key: KeyEvent) -> KeyAction {
    match key.code {
        KeyCode::Esc => KeyAction::ReturnToNormal,
        KeyCode::Enter => KeyAction::Submit,
        _ => KeyAction::InputPassthrough,
    }
}

// ---------------------------------------------------------------------------
// Tier 2: Select mode
// ---------------------------------------------------------------------------

fn dispatch_select(key: KeyEvent) -> KeyAction {
    match key.code {
        KeyCode::Esc => KeyAction::ReturnToNormal,
        KeyCode::Up | KeyCode::Char('k') => KeyAction::ScrollUp,
        KeyCode::Down | KeyCode::Char('j') => KeyAction::ScrollDown,
        KeyCode::Enter => KeyAction::Submit,
        _ => KeyAction::Unhandled,
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
            InteractionMode::Search,
            InteractionMode::Select,
        ];
        for mode in &modes {
            let mut state = AppState::default();
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
        let state = AppState::default(); // focus = Input, mode = Normal
        let action = dispatch(key(KeyCode::Tab), &state);
        assert_eq!(action, KeyAction::CycleFocus);
    }

    // T-TUI-03-03: Enter submits in input-focused normal mode.
    #[test]
    fn test_enter_submits_in_input_normal() {
        let state = AppState::default();
        let action = dispatch(key(KeyCode::Enter), &state);
        assert_eq!(action, KeyAction::Submit);
    }

    // T-TUI-03-04: Shift+Enter passes through as newline in input.
    #[test]
    fn test_shift_enter_passthrough() {
        let state = AppState::default();
        let action = dispatch(key_with_mod(KeyCode::Enter, KeyModifiers::SHIFT), &state);
        assert_eq!(action, KeyAction::InputPassthrough);
    }

    // T-TUI-03-05: j/k scroll in chat-focused normal mode.
    #[test]
    fn test_jk_scroll_chat() {
        let mut state = AppState::default();
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

    // T-TUI-03-06: Ctrl+B toggles sidebar from any panel.
    #[test]
    fn test_ctrl_b_toggles_sidebar() {
        for focus in &[PanelFocus::Input, PanelFocus::Chat, PanelFocus::Sidebar] {
            let mut state = AppState::default();
            state.focus = *focus;
            let action = dispatch(
                key_with_mod(KeyCode::Char('b'), KeyModifiers::CONTROL),
                &state,
            );
            assert_eq!(
                action,
                KeyAction::ToggleSidebar,
                "Ctrl+B should toggle sidebar with {focus:?} focus"
            );
        }
    }

    // T-TUI-03-07: Escape returns to normal from command mode.
    #[test]
    fn test_escape_command_to_normal() {
        let mut state = AppState::default();
        state.mode = InteractionMode::Command;
        let action = dispatch(key(KeyCode::Esc), &state);
        assert_eq!(action, KeyAction::ReturnToNormal);
    }

    // T-TUI-03-08: Colon enters command mode from input.
    #[test]
    fn test_colon_enters_command_mode() {
        let state = AppState::default(); // Input focus, Normal mode
        let action = dispatch(key(KeyCode::Char(':')), &state);
        assert_eq!(action, KeyAction::EnterCommandMode);
    }

    // T-TUI-03-09: Regular chars pass through to textarea in input.
    #[test]
    fn test_char_passthrough_input() {
        let state = AppState::default();
        let action = dispatch(key(KeyCode::Char('a')), &state);
        assert_eq!(action, KeyAction::InputPassthrough);
    }

    // T-TUI-03-10: Escape from search returns to normal.
    #[test]
    fn test_escape_search_to_normal() {
        let mut state = AppState::default();
        state.mode = InteractionMode::Search;
        let action = dispatch(key(KeyCode::Esc), &state);
        assert_eq!(action, KeyAction::ReturnToNormal);
    }

    // T-TUI-03-11: Select mode scrolls with arrows.
    #[test]
    fn test_select_mode_scroll() {
        let mut state = AppState::default();
        state.mode = InteractionMode::Select;
        assert_eq!(dispatch(key(KeyCode::Up), &state), KeyAction::ScrollUp);
        assert_eq!(dispatch(key(KeyCode::Down), &state), KeyAction::ScrollDown);
    }
}
