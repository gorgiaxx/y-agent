//! Panel rendering modules.
//!
//! Each panel is a stateless rendering function that takes `AppState` (read-only)
//! and a `Rect` target area, and renders into a ratatui `Frame`.

pub mod chat;
pub mod input;
pub mod sidebar;
pub mod status_bar;
