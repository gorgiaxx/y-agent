//! Agent listing endpoint.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::state::AppState;

/// Agent info returned in the list.
#[derive(Debug, Serialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// `GET /api/v1/agents` — list registered agent definitions.
async fn list_agents(State(state): State<AppState>) -> Json<Vec<AgentInfo>> {
    let registry = state.container.agent_registry.lock().await;
    let agents: Vec<AgentInfo> = registry
        .list()
        .iter()
        .map(|def| AgentInfo {
            id: def.id.clone(),
            name: def.name.clone(),
            description: def.description.clone(),
        })
        .collect();
    Json(agents)
}

/// Agent route group.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/agents", get(list_agents))
}
