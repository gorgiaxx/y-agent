//! Status bar renderer.
//!
//! Flat powerline-style bar on a subtle background band: foreground-colored
//! segments joined by a thin powerline separator (`\u{E0B1}`, ASCII `›`
//! fallback) in one dim gray:
//!
//! ```text
//! [Left]                                                          [Right]
//! running  session  mode  prompt  path  git  model  ctx (pct%)  $cost   / commands
//! ```

use chrono::Utc;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::keys::{platform_shortcut_label, KeyAction, KeyContext, Keymap};
use crate::tui::panels::chat::SPINNER_FRAMES;
use crate::tui::state::{AppState, InteractionMode};
use crate::tui::theme::Theme;

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

/// Render the status bar into the given area using live data from `AppState`.
///
/// `keymap` drives the contextual shortcut hints in the right segment, so
/// user keymap overrides are reflected in the displayed chords.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState, keymap: &Keymap) {
    let t = &state.theme;
    // Thin powerline separator between segments (one dim gray, no bg blocks);
    // ASCII fallback for terminals without Nerd Font glyphs.
    let sep_glyph = if state.powerline_glyphs {
        "\u{E0B1}"
    } else {
        "\u{203A}"
    };
    let sep = Span::styled(
        format!(" {sep_glyph} "),
        Style::default().fg(t.status_sep()),
    );

    // -- Left section --
    let mut left_spans: Vec<Span> = vec![Span::styled(" ", Style::default())];

    // Always-on running indicator while a response streams.
    let running_spans = build_running_spans(state, t);
    if !running_spans.is_empty() {
        left_spans.extend(running_spans);
        left_spans.push(sep.clone());
    }

    // Session and orchestration mode replace the persistent session sidebar.
    left_spans.push(Span::styled(
        state.current_session_label(),
        Style::default().fg(t.text()).add_modifier(Modifier::BOLD),
    ));
    left_spans.push(sep.clone());
    left_spans.push(Span::styled(
        state.turn_mode.label(),
        Style::default().fg(t.input_border_focused()),
    ));
    left_spans.push(sep.clone());
    left_spans.push(build_permission_status_span(state, t));
    left_spans.push(sep.clone());
    left_spans.push(build_prompt_status_span(state, t));
    if let Some(queue_span) = build_queue_status_span(state, t) {
        left_spans.push(sep.clone());
        left_spans.push(queue_span);
    }
    if let Some(bg_span) = build_bg_task_status_span(state, t) {
        left_spans.push(sep.clone());
        left_spans.push(bg_span);
    }
    if let Some(agents_span) = build_subagent_status_span(state, t) {
        left_spans.push(sep.clone());
        left_spans.push(agents_span);
    }

    // Workspace path (teal) and git working-tree status.
    if let Some(path_span) = build_path_span(state, t) {
        left_spans.push(sep.clone());
        left_spans.push(path_span);
    }
    let git_spans = build_git_spans(state, t);
    if !git_spans.is_empty() {
        left_spans.push(sep.clone());
        left_spans.extend(git_spans);
    }
    left_spans.push(sep.clone());

    // Model name.
    let model_label = if state.status_model.is_empty() {
        "\u{2014}".to_string() // em dash
    } else {
        state.status_model.clone()
    };

    left_spans.push(Span::styled(
        model_label,
        Style::default().fg(t.status_model()),
    ));

    // Context window usage (tokens/window + pct + bar).
    let ctx_spans = build_context_spans(state, t);
    if !ctx_spans.is_empty() {
        left_spans.push(sep.clone());
        left_spans.extend(ctx_spans);
    }

    // Cost.
    if let Some(cost) = state.last_cost {
        if cost > 0.0 {
            left_spans.push(sep.clone());
            left_spans.push(Span::styled(
                format!("${cost:.4}"),
                Style::default().fg(t.status_cost()),
            ));
        }
    }

    // -- Right section --
    // Contextual shortcut hints (streaming also carries a rotating tip);
    // degrades to shorter variants on narrow terminals.
    let total_width = area.width as usize;
    // Display width, not bytes: the bar holds multibyte chars (box-drawing,
    // em dash) that occupy a single cell but multiple bytes.
    let left_len: usize = left_spans.iter().map(Span::width).sum();
    let right_spans = build_right_spans(state, keymap, t, total_width.saturating_sub(left_len));
    let right_len: usize = right_spans.iter().map(Span::width).sum();

    // Fill gap between left and right.
    let gap = total_width.saturating_sub(left_len + right_len);

    let mut spans = left_spans;
    if gap > 0 {
        spans.push(Span::styled(" ".repeat(gap), Style::default()));
    }
    spans.extend(right_spans);

    // Truncate if too wide.
    let total_len: usize = spans.iter().map(Span::width).sum();
    if total_len > total_width && total_width > 3 {
        spans = truncate_spans(spans, total_width, t);
    }

    let line = Line::from(spans);
    // The paragraph base style paints the full-width background band; span
    // styles only patch foreground colors, so the band is uniform.
    let para = Paragraph::new(line).style(Style::default().bg(t.status_bar_bg()));
    frame.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Right segment: contextual shortcut hints + rotating tip
// ---------------------------------------------------------------------------

/// Build the right-aligned segment: contextual shortcut hints for the current
/// interaction context plus, while streaming, the rotating tip.
///
/// Candidates are tried longest-first; the first that fits `avail` columns
/// (including a trailing margin space) wins, so narrow terminals drop the
/// tip first, then secondary hints. Empty when nothing fits.
fn build_right_spans(
    state: &AppState,
    keymap: &Keymap,
    t: &Theme,
    avail: usize,
) -> Vec<Span<'static>> {
    for (hint, tip) in right_candidates(state, keymap) {
        let width = UnicodeWidthStr::width(hint.as_str())
            + tip
                .as_ref()
                .map_or(0, |tip| UnicodeWidthStr::width(tip.as_str()) + 2);
        if width + 1 > avail {
            continue;
        }
        let mut spans = Vec::new();
        if !hint.is_empty() {
            spans.push(Span::styled(hint, Style::default().fg(t.status_version())));
        }
        if let Some(tip) = tip {
            spans.push(Span::styled(tip, Style::default().fg(t.muted())));
        }
        spans.push(Span::styled(" ", Style::default()));
        return spans;
    }
    Vec::new()
}

/// Candidate right segments `(hint, optional "Tip: ..." text)`, longest first.
fn right_candidates(state: &AppState, keymap: &Keymap) -> Vec<(String, Option<String>)> {
    if state.is_streaming {
        let cancel = chord_hint(
            keymap,
            KeyContext::Streaming,
            KeyAction::CancelStreaming,
            "cancel",
        );
        let steer = chord_hint(
            keymap,
            KeyContext::Streaming,
            KeyAction::QueueSteerNext,
            "steer",
        );
        let recall = (!state.follow_up_queue.is_empty())
            .then(|| {
                chord_hint(
                    keymap,
                    KeyContext::Streaming,
                    KeyAction::QueueRecallLast,
                    "edit last",
                )
            })
            .flatten();
        let hints = join_hints(&[cancel.clone(), steer, recall]);
        let tip = format!(
            "Tip: {}",
            crate::tui::tips::tip_for_tick(state.tick_counter)
        );
        let mut candidates = Vec::new();
        if !hints.is_empty() {
            candidates.push((hints.clone(), Some(tip)));
            candidates.push((hints, None));
        }
        if let Some(cancel) = cancel {
            candidates.push((cancel, None));
        }
        return candidates;
    }
    let segment = match state.mode {
        InteractionMode::Shell => join_hints(&[
            chord_hint(keymap, KeyContext::Shell, KeyAction::Submit, "run"),
            chord_hint(
                keymap,
                KeyContext::Shell,
                KeyAction::ReturnToNormal,
                "normal",
            ),
        ]),
        InteractionMode::Command => join_hints(&[
            chord_hint(
                keymap,
                KeyContext::Command,
                KeyAction::CompleteCommand,
                "complete",
            ),
            chord_hint(
                keymap,
                KeyContext::Command,
                KeyAction::ReturnToNormal,
                "close",
            ),
        ]),
        _ => {
            let context = if state.focus == crate::tui::state::PanelFocus::Chat {
                KeyContext::NormalChat
            } else {
                KeyContext::NormalInputEmpty
            };
            join_hints(&[
                chord_hint(keymap, context, KeyAction::RetryLastRequest, "retry"),
                Some("/ commands".to_string()),
                chord_hint(keymap, KeyContext::Global, KeyAction::ShowHelp, "shortcuts"),
                Some(format!("v{}", state.version)),
            ])
        }
    };
    vec![(segment, None)]
}

/// `"<chord> <label>"` for an action in a context, honoring keymap overrides
/// and the host platform's modifier glyphs; `None` when the action is unbound.
fn chord_hint(
    keymap: &Keymap,
    context: KeyContext,
    action: KeyAction,
    label: &str,
) -> Option<String> {
    let chord = keymap.primary_shortcut_in_context(context, action)?;
    Some(format!("{} {label}", platform_shortcut_label(&chord)))
}

/// Join the bound hint parts with a double space, skipping unbound actions.
fn join_hints(parts: &[Option<String>]) -> String {
    parts
        .iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join("  ")
}

fn build_prompt_status_span(state: &AppState, t: &Theme) -> Span<'static> {
    Span::styled(
        state.prompt_template_status.label(),
        Style::default().fg(t.active()),
    )
}

/// Permission-mode segment. Always visible — even `default` — so the active
/// mode is never in doubt; risky modes (bypass/dont-ask) render in warning
/// color so an escalation cannot hide. The `perm:` prefix distinguishes it
/// from the bare orchestration-mode label (`plan` exists in both domains).
fn build_permission_status_span(state: &AppState, t: &Theme) -> Span<'static> {
    use y_core::permission_types::PermissionMode;
    let color = match state.permission_mode {
        PermissionMode::Default => t.muted(),
        PermissionMode::BypassPermissions | PermissionMode::DontAsk => t.warning(),
        PermissionMode::Plan | PermissionMode::AcceptEdits => t.active(),
    };
    Span::styled(
        format!("perm:{}", state.permission_mode),
        Style::default().fg(color),
    )
}

/// Build the always-on "running" segment shown while a response streams.
///
/// The spinner frame is driven by `state.tick_counter` (100 ms ticks) and
/// the elapsed time counts up from the active message's timestamp, so the
/// bar doubles as the at-a-glance "agent is still working" signal.
/// Returns an empty vec when idle.
fn build_running_spans(state: &AppState, t: &Theme) -> Vec<Span<'static>> {
    if !state.is_streaming {
        return Vec::new();
    }
    let frame = SPINNER_FRAMES[(state.tick_counter as usize) % SPINNER_FRAMES.len()];
    let elapsed = state.messages.last().map(|message| {
        let secs = (Utc::now() - message.timestamp).num_seconds().max(0);
        if secs >= 60 {
            format!(" {}m{:02}s", secs / 60, secs % 60)
        } else {
            format!(" {secs}s")
        }
    });
    let label = match elapsed {
        Some(elapsed) => format!("{frame} running{elapsed}"),
        None => format!("{frame} running"),
    };
    vec![Span::styled(
        label,
        Style::default()
            .fg(t.streaming_dot())
            .add_modifier(Modifier::BOLD),
    )]
}

/// Workspace path segment (teal), abbreviated home-relative.
fn build_path_span(state: &AppState, t: &Theme) -> Option<Span<'static>> {
    if state.workspace_dir.is_empty() {
        return None;
    }
    Some(Span::styled(
        abbreviate_path(&state.workspace_dir),
        Style::default().fg(t.status_path()),
    ))
}

/// Abbreviate a directory for the status bar: `$HOME` becomes `~`, and long
/// paths collapse to `…/parent/leaf`.
fn abbreviate_path(dir: &str) -> String {
    const MAX: usize = 30;
    let display = std::env::var("HOME").map_or_else(
        |_| dir.to_string(),
        |home| {
            if !home.is_empty() && dir.starts_with(&home) {
                format!("~{}", &dir[home.len()..])
            } else {
                dir.to_string()
            }
        },
    );
    if display.chars().count() <= MAX {
        return display;
    }
    let mut parts = display.rsplit('/');
    if let (Some(leaf), Some(parent)) = (parts.next(), parts.next()) {
        if !leaf.is_empty() && !parent.is_empty() {
            return format!("…/{parent}/{leaf}");
        }
    }
    display
}

/// Git working-tree segment: branch (green when clean, yellow when dirty)
/// plus per-category change counts, all from the polled cache.
fn build_git_spans(state: &AppState, t: &Theme) -> Vec<Span<'static>> {
    let Some(status) = &state.git_status else {
        return Vec::new();
    };
    if status.branch.is_empty() {
        return Vec::new();
    }
    let branch_color = if status.is_dirty() {
        t.warning()
    } else {
        t.success()
    };
    let mut spans = vec![Span::styled(
        status.branch.clone(),
        Style::default().fg(branch_color),
    )];
    if status.staged > 0 {
        spans.push(Span::styled(
            format!("+{}", status.staged),
            Style::default().fg(t.success()),
        ));
    }
    if status.unstaged > 0 {
        spans.push(Span::styled(
            format!(" *{}", status.unstaged),
            Style::default().fg(t.warning()),
        ));
    }
    if status.untracked > 0 {
        spans.push(Span::styled(
            format!(" ?{}", status.untracked),
            Style::default().fg(t.muted()),
        ));
    }
    spans
}

/// Build the TODO queue depth segment (`todo: N`), visible only while
/// the service-side queue holds pending messages.
fn build_queue_status_span(state: &AppState, t: &Theme) -> Option<Span<'static>> {
    let depth = state.follow_up_queue.len();
    if depth == 0 {
        return None;
    }
    Some(Span::styled(
        format!("todo: {depth}"),
        Style::default().fg(t.active()),
    ))
}

/// Build the background task count segment (`bg: N`), visible only while
/// background shell tasks are running.
fn build_bg_task_status_span(state: &AppState, t: &Theme) -> Option<Span<'static>> {
    if state.bg_task_count == 0 {
        return None;
    }
    Some(Span::styled(
        format!("bg: {}", state.bg_task_count),
        Style::default().fg(t.active()),
    ))
}

/// Build the running subagent count segment (`agents: N`), visible only
/// while one or more subagents are active.
fn build_subagent_status_span(state: &AppState, t: &Theme) -> Option<Span<'static>> {
    if state.active_subagent_count == 0 {
        return None;
    }
    Some(Span::styled(
        format!("agents: {}", state.active_subagent_count),
        Style::default().fg(t.active()),
    ))
}

// ---------------------------------------------------------------------------
// Context window bar (aligned with GUI status-token-bar)
// ---------------------------------------------------------------------------

/// Build styled spans for the context window usage indicator.
///
/// Format: `tokens/window (pct%) [=========-------]`
///
/// Color coding (mirrors pi-powerline-footer thresholds):
/// - Normal (accent) : < 70%
/// - Warning (yellow): >= 70%
/// - Error (red)     : >= 90%
fn build_context_spans(state: &AppState, t: &Theme) -> Vec<Span<'static>> {
    if state.context_window == 0 {
        if state.status_tokens.is_empty() {
            return vec![];
        }
        return vec![Span::styled(
            state.status_tokens.clone(),
            Style::default().fg(t.muted()),
        )];
    }

    let occupancy = state.last_input_tokens;
    if occupancy == 0 {
        return vec![];
    }

    let ctx_window = state.context_window as u64;
    let pct = ((occupancy as f64 / ctx_window as f64) * 100.0).min(100.0);
    let bar_width = 12usize;
    let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
    let empty = bar_width.saturating_sub(filled);

    let level_color = if pct >= 90.0 {
        t.error()
    } else if pct >= 70.0 {
        t.status_bar_warn()
    } else {
        t.status_bar_normal()
    };

    let filled_str: String = "\u{2501}".repeat(filled);
    let empty_str: String = "\u{2500}".repeat(empty);

    let used_label = format_token_count(occupancy);
    let total_label = format_token_count(ctx_window);

    vec![
        Span::styled(
            format!("{used_label}/{total_label}"),
            Style::default().fg(level_color),
        ),
        Span::styled(format!(" ({pct:.1}%) "), Style::default().fg(t.muted())),
        Span::styled(
            filled_str,
            Style::default()
                .fg(level_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(empty_str, Style::default().fg(t.status_bar_track())),
    ]
}

/// Truncate a span list to fit within `max_width` display columns, appending
/// an ellipsis if truncation occurs.
fn truncate_spans(spans: Vec<Span<'static>>, max_width: usize, t: &Theme) -> Vec<Span<'static>> {
    let max = max_width.saturating_sub(1);
    let mut acc = 0;
    let mut result: Vec<Span<'static>> = Vec::new();

    for span in spans {
        let span_width = span.width();
        if acc + span_width <= max {
            result.push(span);
            acc += span_width;
        } else {
            let remaining = max - acc;
            if remaining > 0 {
                let partial = take_display_width(&span.content, remaining);
                result.push(Span::styled(partial, span.style));
            }
            result.push(Span::styled("\u{2026}", Style::default().fg(t.muted())));
            break;
        }
    }

    result
}

/// Take the longest char prefix whose display width fits within `max_width`.
/// A wide char that would straddle the boundary is dropped entirely.
fn take_display_width(value: &str, max_width: usize) -> String {
    let mut width = 0;
    let mut result = String::new();
    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        width += ch_width;
        result.push(ch);
    }
    result
}

// ---------------------------------------------------------------------------
// Token formatting (aligned with GUI formatTokens)
// ---------------------------------------------------------------------------

/// Format a token count for compact display: e.g. 128000 -> "128k", 1500 -> "1.5k".
fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        let m = count as f64 / 1_000_000.0;
        if (m - m.round()).abs() < 0.05 {
            format!("{}M", m.round() as u64)
        } else {
            format!("{m:.1}M")
        }
    } else if count >= 1_000 {
        let k = count as f64 / 1_000.0;
        if (k - k.round()).abs() < 0.05 {
            format!("{}k", k.round() as u64)
        } else {
            format!("{k:.1}k")
        }
    } else {
        format!("{count}")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::keys::Keymap;
    use crate::tui::state::InteractionMode;
    use y_core::permission_types::PermissionMode;

    // T-TUI-STATUS-PERM-DEFAULT: the segment is always visible — even the
    // default mode is shown (muted) so the active mode is never in doubt.
    #[test]
    fn test_permission_segment_shows_default_muted() {
        let state = AppState::new();
        let span = build_permission_status_span(&state, &state.theme);
        assert_eq!(span.content.as_ref(), "perm:default");
        assert_eq!(span.style.fg, Some(state.theme.muted()));
    }

    // T-TUI-STATUS-PERM-RISKY: bypass/dont-ask render in warning color.
    #[test]
    fn test_permission_segment_warning_for_bypass_and_dont_ask() {
        let mut state = AppState::new();
        for mode in [PermissionMode::BypassPermissions, PermissionMode::DontAsk] {
            state.permission_mode = mode;
            let span = build_permission_status_span(&state, &state.theme);
            assert_eq!(span.content.as_ref(), format!("perm:{mode}"));
            assert_eq!(
                span.style.fg,
                Some(state.theme.warning()),
                "{mode} must render in warning color"
            );
        }
    }

    // T-TUI-STATUS-PERM-SOFT: plan/accept-edits render in the active color.
    #[test]
    fn test_permission_segment_active_for_plan_and_accept_edits() {
        let mut state = AppState::new();
        for mode in [PermissionMode::Plan, PermissionMode::AcceptEdits] {
            state.permission_mode = mode;
            let span = build_permission_status_span(&state, &state.theme);
            assert_eq!(span.content.as_ref(), format!("perm:{mode}"));
            assert_eq!(
                span.style.fg,
                Some(state.theme.active()),
                "{mode} must render in the active color"
            );
        }
    }

    fn right_text(state: &AppState, keymap: &Keymap, avail: usize) -> String {
        build_right_spans(state, keymap, &state.theme, avail)
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    // T-TUI-STATUS-HINT-STREAMING: while streaming, the right segment shows
    // the cancel/steer chords plus the rotating tip.
    #[test]
    fn test_right_spans_streaming_show_cancel_steer_and_tip() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.tick_counter = 0;
        let keymap = Keymap::default();

        let text = right_text(&state, &keymap, 120);
        assert!(text.contains("Esc cancel"), "cancel hint: {text}");
        assert!(text.contains("Ctrl+S steer"), "steer hint: {text}");
        assert!(
            text.contains(&format!("Tip: {}", crate::tui::tips::tip_for_tick(0))),
            "rotating tip: {text}"
        );
    }

    // T-TUI-STATUS-HINT-ROTATE: the streaming tip follows the tick counter.
    #[test]
    fn test_right_spans_streaming_tip_rotates_with_ticks() {
        let mut state = AppState::new();
        state.is_streaming = true;
        let keymap = Keymap::default();

        state.tick_counter = 0;
        let first = right_text(&state, &keymap, 120);
        state.tick_counter = crate::tui::tips::TIP_ROTATE_TICKS;
        let second = right_text(&state, &keymap, 120);
        assert_ne!(first, second, "tip must advance after one rotation");
    }

    // T-TUI-STATUS-HINT-NARROW: on narrow bars the tip is dropped first,
    // then the steer hint, keeping the cancel hint as long as possible.
    #[test]
    fn test_right_spans_streaming_narrow_drops_tip_first() {
        let mut state = AppState::new();
        state.is_streaming = true;
        let keymap = Keymap::default();

        // Exactly enough room for both hints (plus trailing space): no tip.
        let hints = "Esc cancel  Ctrl+S steer";
        let text = right_text(&state, &keymap, UnicodeWidthStr::width(hints) + 1);
        assert!(text.contains("Ctrl+S steer"), "hints kept: {text}");
        assert!(!text.contains("Tip:"), "tip dropped: {text}");

        // Only room for the cancel hint.
        let cancel = "Esc cancel";
        let text = right_text(&state, &keymap, UnicodeWidthStr::width(cancel) + 1);
        assert!(text.contains("Esc cancel"), "cancel kept: {text}");
        assert!(!text.contains("steer"), "steer dropped: {text}");

        // No room at all.
        let text = right_text(&state, &keymap, 3);
        assert!(text.is_empty(), "right segment dropped: {text:?}");
    }

    // T-TUI-STATUS-HINT-SHELL: shell mode advertises run/exit chords.
    #[test]
    fn test_right_spans_shell_mode() {
        let mut state = AppState::new();
        state.mode = InteractionMode::Shell;
        let keymap = Keymap::default();

        let text = right_text(&state, &keymap, 120);
        assert!(text.contains("Enter run"), "run hint: {text}");
        assert!(text.contains("Esc normal"), "exit hint: {text}");
    }

    // T-TUI-STATUS-HINT-COMMAND: command mode advertises completion chords.
    #[test]
    fn test_right_spans_command_mode() {
        let mut state = AppState::new();
        state.mode = InteractionMode::Command;
        let keymap = Keymap::default();

        let text = right_text(&state, &keymap, 120);
        assert!(text.contains("Tab complete"), "complete hint: {text}");
        assert!(text.contains("Esc close"), "close hint: {text}");
    }

    // T-TUI-STATUS-HINT-DEFAULT: idle normal mode keeps the legacy segment.
    #[test]
    fn test_right_spans_default_shows_commands_and_version() {
        let state = AppState::new();
        let keymap = Keymap::default();

        let text = right_text(&state, &keymap, 120);
        assert!(text.contains("/ commands"), "commands hint: {text}");
        assert!(text.contains(&state.version), "version kept: {text}");
    }

    // T-TUI-STATUS-HINT-KEYMAP: hints derive from the live keymap, so user
    // overrides change the displayed chord.
    #[test]
    fn test_right_spans_respects_keymap_override() {
        let mut state = AppState::new();
        state.is_streaming = true;
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("cancel_streaming".to_string(), vec!["ctrl+x".to_string()]);
        let keymap = Keymap::with_overrides(overrides).unwrap();

        let text = right_text(&state, &keymap, 120);
        assert!(text.contains("Ctrl+X cancel"), "override chord: {text}");
        assert!(!text.contains("Esc cancel"), "default chord gone: {text}");
    }

    #[test]
    fn test_format_token_count_small() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(999), "999");
    }

    // T-TUI-STATUS-BG: the bar paints its background band across the full row,
    // including cells past the last segment.
    #[test]
    fn test_render_paints_background_band_full_width() {
        let state = AppState::new();
        let expected_bg = state.theme.status_bar_bg();

        let backend = ratatui::backend::TestBackend::new(60, 3);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state, &Keymap::default()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        for x in 0..60 {
            for y in 0..3 {
                let cell = buffer.cell((x, y)).unwrap();
                assert_eq!(
                    cell.bg, expected_bg,
                    "cell ({x}, {y}) must carry the status bar background"
                );
            }
        }
    }

    #[test]
    fn test_format_token_count_thousands() {
        assert_eq!(format_token_count(1_000), "1k");
        assert_eq!(format_token_count(1_500), "1.5k");
        assert_eq!(format_token_count(128_000), "128k");
        assert_eq!(format_token_count(200_000), "200k");
    }

    #[test]
    fn test_format_token_count_millions() {
        assert_eq!(format_token_count(1_000_000), "1M");
        assert_eq!(format_token_count(1_500_000), "1.5M");
    }

    #[test]
    fn test_build_context_spans_zero_window() {
        let state = AppState::new();
        let spans = build_context_spans(&state, &state.theme);
        assert!(spans.is_empty(), "no spans when window is 0 and no tokens");
    }

    #[test]
    fn test_prompt_template_status_span_is_always_visible() {
        let mut state = AppState::new();
        assert_eq!(
            build_prompt_status_span(&state, &state.theme)
                .content
                .as_ref(),
            "prompt:default"
        );

        state.prompt_template_status = crate::tui::state::PromptTemplateStatus::Template {
            id: "review".into(),
            name: "Reviewer".into(),
        };
        assert_eq!(
            build_prompt_status_span(&state, &state.theme)
                .content
                .as_ref(),
            "prompt:Reviewer"
        );
    }

    #[test]
    fn test_running_spans_hidden_when_idle() {
        let state = AppState::new();
        assert!(build_running_spans(&state, &state.theme).is_empty());
    }

    #[test]
    fn test_running_spans_show_spinner_frame_and_follow_tick() {
        let mut state = AppState::new();
        state.is_streaming = true;

        let spans = build_running_spans(&state, &state.theme);
        assert_eq!(spans.len(), 1);
        let text = spans[0].content.as_ref();
        assert!(text.contains("running"), "expected running label: {text}");
        assert!(
            text.contains(SPINNER_FRAMES[0]),
            "tick 0 must show the first frame: {text}"
        );

        // The animation tick drives the frame.
        state.tick_counter = 1;
        let spans = build_running_spans(&state, &state.theme);
        let text = spans[0].content.as_ref();
        assert!(text.contains(SPINNER_FRAMES[1]), "tick 1 frame: {text}");

        // Frame index wraps around the frame table.
        state.tick_counter = SPINNER_FRAMES.len() as u64;
        let spans = build_running_spans(&state, &state.theme);
        assert!(spans[0].content.contains(SPINNER_FRAMES[0]));
    }

    fn follow_up(text: &str) -> y_service::FollowUpMessage {
        y_service::FollowUpMessage {
            id: format!("fu-{text}"),
            text: text.to_string(),
            created_at: 0,
            status: y_service::FollowUpStatus::default(),
        }
    }

    #[test]
    fn test_queue_status_span_hidden_when_queue_empty() {
        let state = AppState::new();
        assert!(build_queue_status_span(&state, &state.theme).is_none());
    }

    #[test]
    fn test_queue_status_span_shows_pending_count() {
        let mut state = AppState::new();
        state.follow_up_queue.push(follow_up("one"));
        state.follow_up_queue.push(follow_up("two"));

        let span = build_queue_status_span(&state, &state.theme).unwrap();
        assert_eq!(span.content.as_ref(), "todo: 2");
    }

    #[test]
    fn test_bg_and_agents_spans_hidden_at_zero() {
        let state = AppState::new();
        assert!(build_bg_task_status_span(&state, &state.theme).is_none());
        assert!(build_subagent_status_span(&state, &state.theme).is_none());
    }

    #[test]
    fn test_bg_task_span_shows_running_count() {
        let mut state = AppState::new();
        state.bg_task_count = 2;

        let span = build_bg_task_status_span(&state, &state.theme).unwrap();
        assert_eq!(span.content.as_ref(), "bg: 2");
        // The subagent segment stays hidden while its count is zero.
        assert!(build_subagent_status_span(&state, &state.theme).is_none());
    }

    #[test]
    fn test_subagent_span_shows_running_count() {
        let mut state = AppState::new();
        state.active_subagent_count = 3;

        let span = build_subagent_status_span(&state, &state.theme).unwrap();
        assert_eq!(span.content.as_ref(), "agents: 3");
        // The background task segment stays hidden while its count is zero.
        assert!(build_bg_task_status_span(&state, &state.theme).is_none());
    }

    #[test]
    fn test_bg_and_agents_spans_shown_together() {
        let mut state = AppState::new();
        state.bg_task_count = 1;
        state.active_subagent_count = 4;

        assert_eq!(
            build_bg_task_status_span(&state, &state.theme)
                .unwrap()
                .content
                .as_ref(),
            "bg: 1"
        );
        assert_eq!(
            build_subagent_status_span(&state, &state.theme)
                .unwrap()
                .content
                .as_ref(),
            "agents: 4"
        );
    }

    #[test]
    fn test_build_context_spans_with_usage() {
        let mut state = AppState::new();
        state.context_window = 128_000;
        state.last_input_tokens = 64_000;
        let spans = build_context_spans(&state, &state.theme);
        // Should have 4 spans: ratio, pct, filled bar, empty bar.
        assert_eq!(spans.len(), 4);
        // Ratio should contain token counts.
        let ratio = &spans[0].content;
        assert!(ratio.contains("64k"), "expected 64k in ratio, got: {ratio}");
        assert!(
            ratio.contains("128k"),
            "expected 128k in ratio, got: {ratio}"
        );
        // Pct should contain percentage.
        let pct_span = &spans[1].content;
        assert!(
            pct_span.contains("50.0%"),
            "expected 50.0% in pct, got: {pct_span}"
        );
    }

    #[test]
    fn test_context_color_coding() {
        let mut state = AppState::new();
        state.context_window = 100;
        let t = &state.theme;

        // < 80% -> normal accent color
        state.last_input_tokens = 30;
        let spans = build_context_spans(&state, t);
        assert_eq!(spans[2].style.fg, Some(t.status_bar_normal()));

        // >= 80% -> warning color
        state.last_input_tokens = 85;
        let spans = build_context_spans(&state, t);
        assert_eq!(spans[2].style.fg, Some(t.status_bar_warn()));
    }

    #[test]
    fn test_truncate_spans_short() {
        let t = Theme::default();
        let spans = vec![Span::raw("hello")];
        let result = truncate_spans(spans, 10, &t);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content.as_ref(), "hello");
    }

    #[test]
    fn test_truncate_spans_overflow() {
        let t = Theme::default();
        let spans = vec![Span::raw("hello world this is long")];
        let result = truncate_spans(spans, 10, &t);
        // Should truncate and add ellipsis.
        assert!(result.len() >= 2);
        let total_chars: usize = result.iter().map(|s| s.content.chars().count()).sum();
        assert!(
            total_chars <= 10,
            "truncated result too long: {total_chars} chars",
        );
    }

    #[test]
    fn test_truncate_spans_keeps_multibyte_bar_that_fits() {
        let t = Theme::default();
        // 12 box-drawing chars: 12 display cells but 36 bytes. A byte-counted
        // implementation would wrongly truncate this.
        let bar: String = "\u{2501}".repeat(6) + &"\u{2500}".repeat(6);
        let spans = vec![Span::raw(bar.clone())];
        let result = truncate_spans(spans, 13, &t);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content.as_ref(), bar.as_str());
    }

    #[test]
    fn test_truncate_spans_cuts_by_display_width() {
        let t = Theme::default();
        // Each CJK char is 2 cells wide; width budget 7 allows 3 chars (6
        // cells) plus the ellipsis.
        let spans = vec![Span::raw("你好世界再")];
        let result = truncate_spans(spans, 7, &t);
        let total_width: usize = result.iter().map(Span::width).sum();
        assert!(
            total_width <= 7,
            "truncated result exceeds width budget: {total_width}"
        );
        assert_eq!(result[0].content.as_ref(), "你好世");
        assert_eq!(result[1].content.as_ref(), "\u{2026}");
    }

    #[test]
    fn test_take_display_width_drops_straddling_wide_char() {
        // 3 ASCII chars (3 cells) + 1 CJK char (2 cells) = 5 cells; a budget
        // of 4 must drop the wide char instead of splitting it.
        assert_eq!(take_display_width("abc好de", 4), "abc");
        assert_eq!(take_display_width("abc好de", 5), "abc好");
        assert_eq!(take_display_width("ab", 10), "ab");
    }

    #[test]
    fn test_git_spans_hidden_without_status() {
        let state = AppState::new();
        assert!(build_git_spans(&state, &state.theme).is_empty());
    }

    #[test]
    fn test_git_spans_branch_color_and_counts() {
        let mut state = AppState::new();

        state.git_status = Some(crate::tui::git_status::GitStatus {
            branch: "main".into(),
            staged: 0,
            unstaged: 0,
            untracked: 0,
        });
        let spans = build_git_spans(&state, &state.theme);
        assert_eq!(spans.len(), 1, "clean tree shows the branch only");
        assert_eq!(spans[0].content.as_ref(), "main");
        assert_eq!(spans[0].style.fg, Some(state.theme.success()));

        state.git_status = Some(crate::tui::git_status::GitStatus {
            branch: "main".into(),
            staged: 2,
            unstaged: 1,
            untracked: 3,
        });
        let spans = build_git_spans(&state, &state.theme);
        let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(text, "main+2 *1 ?3");
        assert_eq!(spans[0].style.fg, Some(state.theme.warning()));
        assert_eq!(spans[1].style.fg, Some(state.theme.success()));
        assert_eq!(spans[2].style.fg, Some(state.theme.warning()));
        assert_eq!(spans[3].style.fg, Some(state.theme.muted()));
    }

    #[test]
    fn test_path_segment_uses_teal_and_abbreviates() {
        let mut state = AppState::new();
        state.workspace_dir = String::new();
        assert!(build_path_span(&state, &state.theme).is_none());

        state.workspace_dir = "/tmp/work".into();
        let span = build_path_span(&state, &state.theme).unwrap();
        assert_eq!(span.style.fg, Some(state.theme.status_path()));
        assert_eq!(span.content.as_ref(), "/tmp/work");

        let long = format!("/{}/leaf", "a".repeat(40));
        let abbreviated = abbreviate_path(&long);
        assert!(
            abbreviated.starts_with('…'),
            "long paths collapse to a tail: {abbreviated}"
        );
        assert!(abbreviated.ends_with("/leaf"));
    }

    #[test]
    fn test_running_spans_include_elapsed_time() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(crate::tui::state::ChatMessage {
            role: crate::tui::state::MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now() - chrono::Duration::seconds(65),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        });

        let spans = build_running_spans(&state, &state.theme);
        let text = spans[0].content.as_ref();
        assert!(text.contains("running"), "expected running label: {text}");
        assert!(text.contains("1m05s"), "expected elapsed minutes: {text}");
    }

    #[test]
    fn test_context_error_threshold_at_90_percent() {
        let mut state = AppState::new();
        state.context_window = 100;
        let t = &state.theme;

        state.last_input_tokens = 75;
        let spans = build_context_spans(&state, t);
        assert_eq!(spans[2].style.fg, Some(t.status_bar_warn()));

        state.last_input_tokens = 95;
        let spans = build_context_spans(&state, t);
        assert_eq!(spans[2].style.fg, Some(t.error()));
        assert_eq!(spans[0].style.fg, Some(t.error()));
    }
}
