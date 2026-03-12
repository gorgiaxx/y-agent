//! Tool listing endpoint.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::state::AppState;

/// Tool info returned in the list.
#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// `GET /api/v1/tools` — list registered tools.
async fn list_tools(State(state): State<AppState>) -> Json<Vec<ToolInfo>> {
    let defs = state.container.tool_registry.get_all_definitions().await;
    let tools: Vec<ToolInfo> = defs
        .iter()
        .map(|def| ToolInfo {
            name: def.name.as_str().to_string(),
            description: def.description.clone(),
            parameters: def.parameters.clone(),
        })
        .collect();
    Json(tools)
}

/// Tool route group.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/tools", get(list_tools))
}
