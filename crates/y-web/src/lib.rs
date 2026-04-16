//! y-web: HTTP REST API server for y-agent.
//!
//! This crate provides an axum-based HTTP server as a presentation layer
//! on top of `y-service::ServiceContainer`. It exposes the same business
//! logic as CLI, TUI, and GUI through `RESTful` endpoints plus SSE streaming.
//!
//! ## Architecture
//!
//! ```text
//! HTTP Client  ->  axum Router  ->  handlers  ->  y-service  ->  domain crates
//!              <-  SSE stream   <-  broadcast  <-  (async events)
//! ```

pub mod error;
pub mod routes;
pub mod state;

pub use routes::create_router;
pub use state::{AppState, KnowledgeState, WebConfig};
