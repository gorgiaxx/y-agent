//! Key dispatcher: maps key events to state transitions based on mode and focus.
//!
//! The dispatcher follows a two-tier priority:
//! 1. **Global keys** (Ctrl+Q/D/C, F1, Ctrl+O) — always handled, regardless of mode/focus.
//! 2. **Mode + Focus keys** — dispatched based on `InteractionMode` × `PanelFocus`.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::tui::state::{AppState, InteractionMode, PanelFocus};

/// Result of dispatching a key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Enter command mode.
    EnterCommandMode,
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
    /// Remove the selected follow-up from the queue.
    QueueDelete,
    /// Promote the selected follow-up to the pending steer (or demote it).
    QueueSteer,
    /// Kill the selected entry in the `/tasks` overlay.
    TasksKill,
    /// Refresh the `/tasks` overlay contents.
    TasksRefresh,
}

/// Dispatch a key event against the current state.
///
/// Returns a `KeyAction` indicating what the caller should do.
pub fn dispatch(key: KeyEvent, state: &AppState) -> KeyAction {
    // Only react to key presses. Platforms/terminals that emit Release or
    // Repeat events (e.g. Windows, Kitty keyboard protocol) would otherwise
    // dispatch a single keystroke more than once.
    if key.kind != KeyEventKind::Press {
        return KeyAction::Consumed;
    }

    // Tier 1: Global shortcuts (always active).
    if let Some(action) = dispatch_global(key) {
        return action;
    }

    // Tier 2: Mode-specific dispatch.
    match state.mode {
        InteractionMode::Normal => dispatch_normal(key, state),
        InteractionMode::Command => dispatch_command(key, state),
        InteractionMode::Select => dispatch_select(key),
        InteractionMode::Help => dispatch_help(key),
        InteractionMode::Queue => dispatch_queue(key),
        InteractionMode::Tasks => dispatch_tasks(key),
        InteractionMode::Copy | InteractionMode::Resume | InteractionMode::Prompt => {
            dispatch_picker(key)
        }
    }
}

// ---------------------------------------------------------------------------
// Tier 1: Global keys
// ---------------------------------------------------------------------------

fn dispatch_global(key: KeyEvent) -> Option<KeyAction> {
    // F1 shows help globally. Ctrl+H is intentionally NOT bound: many
    // terminals send ^H (0x08) for Backspace, so binding it would pop the
    // help overlay every time the user deletes a character.
    if key.code == KeyCode::F(1) {
        return Some(KeyAction::ShowHelp);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char('q' | 'd' | 'c') = key.code {
            return Some(KeyAction::Quit);
        }
        // Ctrl+O cycles the selected tool card from any mode/focus. It is
        // global (not chat-focus-only like Enter) because the common case is
        // expanding the latest tool card while typing in the composer. No
        // picker or overlay binds Ctrl+O, so this is conflict-free.
        if key.code == KeyCode::Char('o') {
            return Some(KeyAction::ToggleSelectedTool);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tier 2: Normal mode
// ---------------------------------------------------------------------------

fn dispatch_normal(key: KeyEvent, state: &AppState) -> KeyAction {
    match state.focus {
        PanelFocus::Input => dispatch_input_normal(key, state),
        PanelFocus::Chat => dispatch_chat_normal(key, state),
    }
}

/// Normal mode, Input panel focused.
fn dispatch_input_normal(key: KeyEvent, state: &AppState) -> KeyAction {
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
        // Ctrl+G scrolls to bottom (dismiss "new content below").
        _ if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('g') => {
            KeyAction::ScrollToBottom
        }
        // ':' prefix enters command mode.
        KeyCode::Char(':') => KeyAction::EnterCommandMode,
        // Escape cancels streaming if active, otherwise opens prompt backtracking.
        KeyCode::Esc => {
            if state.is_cancelling {
                KeyAction::Consumed
            } else if state.is_streaming {
                KeyAction::CancelStreaming
            } else {
                KeyAction::EnterBacktrack
            }
        }
        // Everything else passes through to textarea.
        _ => KeyAction::InputPassthrough,
    }
}

/// Normal mode, Chat panel focused.
fn dispatch_chat_normal(key: KeyEvent, state: &AppState) -> KeyAction {
    match key.code {
        KeyCode::Char(']') => KeyAction::SelectNextTool,
        KeyCode::Char('[') => KeyAction::SelectPreviousTool,
        KeyCode::Enter => KeyAction::ToggleSelectedTool,
        KeyCode::Char('c') => KeyAction::CopySelectedTool,
        // Line scroll.
        KeyCode::Up | KeyCode::Char('k') => KeyAction::ScrollUp,
        KeyCode::Down | KeyCode::Char('j') => KeyAction::ScrollDown,
        // Page scroll.
        KeyCode::PageUp => KeyAction::PageScrollUp,
        KeyCode::PageDown => KeyAction::PageScrollDown,
        // Jump to top/bottom.
        KeyCode::Home | KeyCode::Char('g') => KeyAction::ScrollToTop,
        KeyCode::End | KeyCode::Char('G') => KeyAction::ScrollToBottom,
        // Tab cycles focus.
        KeyCode::Tab => KeyAction::CycleFocus,
        // '?' shows help.
        KeyCode::Char('?') => KeyAction::ShowHelp,
        // Escape cancels streaming if active, otherwise opens prompt backtracking.
        KeyCode::Esc => {
            if state.is_cancelling {
                KeyAction::Consumed
            } else if state.is_streaming {
                KeyAction::CancelStreaming
            } else {
                KeyAction::EnterBacktrack
            }
        }
        // 'i' returns focus to input.
        KeyCode::Char('i') => KeyAction::ReturnToNormal,
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
        KeyCode::Down | KeyCode::Tab => KeyAction::ScrollDown,
        // Everything else is input for the command buffer.
        _ => KeyAction::InputPassthrough,
    }
}

// ---------------------------------------------------------------------------
// Tier 2: Select mode
// ---------------------------------------------------------------------------

fn dispatch_select(key: KeyEvent) -> KeyAction {
    match key.code {
        // Escape exits backtrack selection, consistent with other modes.
        KeyCode::Esc | KeyCode::Char('q' | 'i') => KeyAction::ReturnToNormal,
        KeyCode::Up | KeyCode::Left | KeyCode::Char('k') => KeyAction::BacktrackPrevious,
        KeyCode::Down | KeyCode::Right | KeyCode::Char('j') => KeyAction::BacktrackNext,
        KeyCode::Enter => KeyAction::ConfirmBacktrack,
        _ => KeyAction::Unhandled,
    }
}

// ---------------------------------------------------------------------------
// Tier 2: Help mode
// ---------------------------------------------------------------------------

fn dispatch_help(key: KeyEvent) -> KeyAction {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => KeyAction::ReturnToNormal,
        _ => KeyAction::Consumed,
    }
}

fn dispatch_picker(key: KeyEvent) -> KeyAction {
    match key.code {
        KeyCode::Esc => KeyAction::ReturnToNormal,
        KeyCode::Enter => KeyAction::Submit,
        KeyCode::Up => KeyAction::ScrollUp,
        KeyCode::Down | KeyCode::Tab => KeyAction::ScrollDown,
        KeyCode::PageUp => KeyAction::PageScrollUp,
        KeyCode::PageDown => KeyAction::PageScrollDown,
        _ => KeyAction::InputPassthrough,
    }
}

// ---------------------------------------------------------------------------
// Tier 2: Queue mode
// ---------------------------------------------------------------------------

fn dispatch_queue(key: KeyEvent) -> KeyAction {
    match key.code {
        // Escape or q closes the queue overlay.
        KeyCode::Esc | KeyCode::Char('q') => KeyAction::ReturnToNormal,
        // Enter closes the overlay; queue mutations use d/s instead.
        KeyCode::Enter => KeyAction::Submit,
        // Arrow / vim keys navigate the queue.
        KeyCode::Up | KeyCode::Char('k') => KeyAction::ScrollUp,
        KeyCode::Down | KeyCode::Char('j') => KeyAction::ScrollDown,
        KeyCode::Char('d') => KeyAction::QueueDelete,
        KeyCode::Char('s') => KeyAction::QueueSteer,
        // Everything else is swallowed so it cannot leak into the composer.
        _ => KeyAction::Consumed,
    }
}

// ---------------------------------------------------------------------------
// Tier 2: Tasks mode
// ---------------------------------------------------------------------------

fn dispatch_tasks(key: KeyEvent) -> KeyAction {
    match key.code {
        // Escape or q closes the tasks overlay.
        KeyCode::Esc | KeyCode::Char('q') => KeyAction::ReturnToNormal,
        // Enter toggles the selected task's inline output preview.
        KeyCode::Enter => KeyAction::Submit,
        // Arrow / vim keys navigate the list, consistent with the queue
        // overlay; kill lives on `d` there too (delete).
        KeyCode::Up | KeyCode::Char('k') => KeyAction::ScrollUp,
        KeyCode::Down | KeyCode::Char('j') => KeyAction::ScrollDown,
        KeyCode::Char('d') => KeyAction::TasksKill,
        KeyCode::Char('r') => KeyAction::TasksRefresh,
        // Everything else is swallowed so it cannot leak into the composer.
        _ => KeyAction::Consumed,
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
            KeyAction::ReturnToNormal
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
}
