//! Async event loop for the TUI.
//!
//! Multiplexes crossterm terminal events (key presses, resize) with an
//! internal tick timer for periodic UI updates (e.g., streaming frame batching).

use std::time::Duration;

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};
use tokio::sync::mpsc;
use tokio::time::interval;

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
    /// The terminal was resized.
    Resize(u16, u16),
    /// Periodic tick for UI refresh (frame batching, animations).
    Tick,
}

// ---------------------------------------------------------------------------
// EventLoop
// ---------------------------------------------------------------------------

/// Async event loop that combines crossterm events with a tick timer.
///
/// Runs in a background task, sending `AppEvent` values through a channel
/// that the main TUI loop consumes.
pub struct EventLoop {
    /// Receiving end of the event channel.
    rx: mpsc::UnboundedReceiver<AppEvent>,
}

impl EventLoop {
    /// Create and start the event loop.
    ///
    /// Spawns a background tokio task that polls crossterm events and emits
    /// ticks. The tick interval controls the minimum UI refresh rate.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut tick = interval(tick_rate);
            // Track last resize for debouncing (50ms window).
            let mut last_resize: Option<(u16, u16, tokio::time::Instant)> = None;
            let resize_debounce = Duration::from_millis(50);

            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        // Emit debounced resize if pending.
                        if let Some((w, h, when)) = last_resize.take() {
                            if when.elapsed() >= resize_debounce {
                                if tx.send(AppEvent::Resize(w, h)).is_err() {
                                    break;
                                }
                            } else {
                                // Put it back; not ready yet.
                                last_resize = Some((w, h, when));
                            }
                        }

                        if tx.send(AppEvent::Tick).is_err() {
                            break;
                        }
                    }
                    // Poll crossterm events with a small timeout so we can
                    // service the tick timer too.
                    _ = tokio::task::spawn_blocking(|| event::poll(Duration::from_millis(10))) => {
                        if let Ok(true) = event::poll(Duration::ZERO) {
                            if let Ok(evt) = event::read() {
                                match evt {
                                    CrosstermEvent::Key(key) => {
                                        if tx.send(AppEvent::Key(key)).is_err() {
                                            break;
                                        }
                                    }
                                    CrosstermEvent::Mouse(mouse) => {
                                        if tx.send(AppEvent::Mouse(mouse)).is_err() {
                                            break;
                                        }
                                    }
                                    CrosstermEvent::Resize(w, h) => {
                                        // Debounce resizes: store latest, emit on tick.
                                        last_resize = Some((w, h, tokio::time::Instant::now()));
                                    }
                                    _ => {} // Paste, etc. — ignored for now.
                                }
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
