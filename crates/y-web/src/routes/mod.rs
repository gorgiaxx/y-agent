//! Route modules and router construction.

pub mod agents;
pub mod attachments;
pub mod bots;
pub mod chat;
pub mod config;
pub mod diagnostics;
pub mod events;
pub mod health;
pub mod knowledge;
pub mod observability;
pub mod rewind;
pub mod schedules;
pub mod sessions;
pub mod skills;
pub mod tools;
pub mod workflows;
pub mod workspaces;

use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Build the full application router with all route groups.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .merge(health::router())
        .merge(sessions::router())
        .merge(chat::router())
        .merge(agents::router())
        .merge(tools::router())
        .merge(diagnostics::router())
        .merge(bots::router())
        .merge(workflows::router())
        .merge(schedules::router())
        .merge(events::router())
        .merge(config::router())
        .merge(workspaces::router())
        .merge(skills::router())
        .merge(knowledge::router())
        .merge(observability::router())
        .merge(rewind::router())
        .merge(attachments::router())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}
