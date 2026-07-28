//! Panel rendering modules.
//!
//! Each panel is a rendering function that takes `AppState` (read-only) and a
//! `Rect` target area, and renders into a ratatui `Frame`. The chat panel
//! additionally takes a per-message render cache and a plain-lines out-buffer
//! so frames only recompute what changed.

pub mod chat;
pub mod input;
pub mod status_bar;
