//! y-web: HTTP REST API server for y-agent.
//!
//! This crate provides an axum-based HTTP server as a presentation layer
//! on top of `y-service::ServiceContainer`. It exposes the same business
//! logic as CLI and TUI through `RESTful` endpoints.
//!
//! ## Architecture
//!
//! ```text
//! HTTP Client  →  axum Router  →  handlers  →  y-service  →  domain crates
//! ```
//!
//! ## Quick Start
//!
//! ```ignore
//! use y_web::{AppState, create_router, WebConfig};
//! use y_service::{ServiceConfig, ServiceContainer};
//! use std::sync::Arc;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let container = ServiceContainer::from_config(&ServiceConfig::default()).await?;
//! let state = AppState::new(Arc::new(container), env!("CARGO_PKG_VERSION"));
//! let app = create_router(state);
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//! axum::serve(listener, app).await?;
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod routes;
pub mod state;

pub use routes::create_router;
pub use state::{AppState, WebConfig};
