//! Async event loop for the TUI.
//!
//! Multiplexes crossterm terminal events (key presses, resize) with an
//! internal tick timer for periodic UI updates (e.g., streaming frame batching).
//!
//! A dedicated blocking thread performs `crossterm::event::read()` and pushes
//! raw events into a channel, so the async side never burns tasks on
//! `spawn_blocking` polls. Resizes are debounced on a dedicated short timer
//! instead of the tick, keeping resize latency around 50ms regardless of the
//! tick rate.

use std::time::Duration;

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};
use tokio::sync::mpsc;
use tokio::time::{interval, Instant};

/// Delay applied to resize events so a burst of resizes collapses into one.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
// AppEvent
// ---------------------------------------------------------------------------

/// Events processed by the TUI main loop.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A key was pressed.
    Key(KeyEvent),
    /// A mouse event occurred.
    Mouse(crossterm::event::MouseEvent),
    /// Bracketed paste: the terminal delivered the pasted text as one event.
    Paste(String),
    /// The terminal was resized.
    Resize(u16, u16),
    /// Periodic tick for UI refresh (frame batching, animations).
    Tick,
}

/// Map a raw crossterm event to an `AppEvent`, when one should be emitted.
///
/// Returns `None` for resizes (debounced separately by the event loop) and
/// for events the TUI does not consume (focus changes, etc.).
fn map_terminal_event(evt: &CrosstermEvent) -> Option<AppEvent> {
    match evt {
        CrosstermEvent::Key(key) => Some(AppEvent::Key(*key)),
        CrosstermEvent::Mouse(mouse) => Some(AppEvent::Mouse(*mouse)),
        CrosstermEvent::Paste(text) => Some(AppEvent::Paste(text.clone())),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// EventLoop
// ---------------------------------------------------------------------------

/// Async event loop that combines crossterm events with a tick timer.
///
/// A background blocking thread reads crossterm events and a background
/// tokio task multiplexes them with ticks, sending `AppEvent` values through
/// a channel that the main TUI loop consumes.
pub struct EventLoop {
    /// Receiving end of the event channel.
    rx: mpsc::UnboundedReceiver<AppEvent>,
}

impl EventLoop {
    /// Create and start the event loop.
    ///
    /// Spawns a blocking thread that reads crossterm events as they arrive,
    /// plus a tokio task that merges those events with the tick timer. The
    /// tick interval controls the minimum UI refresh rate.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        // Raw crossterm events flow from the blocking reader thread into the
        // async multiplexer. `UnboundedSender::send` is synchronous and
        // non-blocking, so no per-event task spawning is needed.
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<CrosstermEvent>();
        std::thread::spawn(move || {
            // Exits when the terminal read fails or the receiver is dropped
            // (TUI shutdown).
            while let Ok(evt) = event::read() {
                if event_tx.send(evt).is_err() {
                    break;
                }
            }
        });

        tokio::spawn(async move {
            let mut tick = interval(tick_rate);
            // Latest resize awaiting emission, debounced on its own timer so
            // resizes are delivered within ~50ms instead of waiting for a tick.
            let mut pending_resize: Option<(u16, u16)> = None;
            let resize_timer = tokio::time::sleep(RESIZE_DEBOUNCE);
            tokio::pin!(resize_timer);

            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        if tx.send(AppEvent::Tick).is_err() {
                            break;
                        }
                    }
                    Some(evt) = event_rx.recv() => {
                        if let CrosstermEvent::Resize(w, h) = evt {
                            // Keep only the latest resize and restart the
                            // debounce window.
                            pending_resize = Some((w, h));
                            resize_timer
                                .as_mut()
                                .reset(Instant::now() + RESIZE_DEBOUNCE);
                        } else if let Some(app_event) = map_terminal_event(&evt) {
                            if tx.send(app_event).is_err() {
                                break;
                            }
                        }
                    }
                    () = &mut resize_timer, if pending_resize.is_some() => {
                        if let Some((w, h)) = pending_resize.take() {
                            if tx.send(AppEvent::Resize(w, h)).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });

        Self { rx }
    }

    /// Wait for the next event from the loop.
    ///
    /// Returns `None` if the event loop task has been dropped.
    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    // The loop must emit ticks even when no terminal events arrive (e.g.
    // when stdin is not a TTY and the reader thread has nothing to read).
    #[tokio::test]
    async fn test_event_loop_emits_ticks() {
        let mut events = EventLoop::new(Duration::from_millis(5));
        let event = tokio::time::timeout(Duration::from_secs(1), events.next())
            .await
            .expect("event loop should emit a tick promptly");
        assert!(matches!(event, Some(AppEvent::Tick)));
    }

    #[test]
    fn test_map_terminal_event_surfaces_paste() {
        let event = map_terminal_event(&CrosstermEvent::Paste("hello\nworld".to_string()));
        assert!(
            matches!(event, Some(AppEvent::Paste(text)) if text == "hello\nworld"),
            "bracketed paste must reach the main loop with text intact"
        );
    }

    #[test]
    fn test_map_terminal_event_maps_key_and_mouse() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(matches!(
            map_terminal_event(&CrosstermEvent::Key(key)),
            Some(AppEvent::Key(_))
        ));
    }

    #[test]
    fn test_map_terminal_event_skips_resize_and_unhandled() {
        // Resizes are debounced by the event loop, not mapped here.
        assert!(map_terminal_event(&CrosstermEvent::Resize(80, 24)).is_none());
        assert!(map_terminal_event(&CrosstermEvent::FocusGained).is_none());
    }
}
