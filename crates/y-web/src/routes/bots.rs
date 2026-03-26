//! Bot webhook routes.
//!
//! Provides HTTP endpoints for receiving inbound events from bot platforms
//! (Feishu, Discord, Telegram). Events are verified, parsed, and then processed
//! asynchronously via [`BotService`].

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use tracing::{error, info, warn};

use y_bot::BotPlatform;
use y_service::BotService;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Feishu webhook handler
// ---------------------------------------------------------------------------

/// `POST /api/v1/bots/feishu/webhook` — receive Feishu event callbacks.
///
/// Feishu requires webhooks to respond within 3 seconds, so we spawn
/// message processing asynchronously and return 200 immediately.
async fn feishu_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(ref feishu_bot) = state.feishu_bot else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Feishu bot not configured" })),
        );
    };

    // Convert headers to HashMap<String, String> (lowercase keys).
    let header_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|val| (k.as_str().to_lowercase(), val.to_string()))
        })
        .collect();

    // 1. Verify signature.
    if let Err(e) = feishu_bot.verify_signature(&header_map, &body) {
        warn!(error = %e, "Feishu webhook: signature verification failed");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid signature" })),
        );
    }

    // 2. Parse JSON body.
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "Feishu webhook: invalid JSON body");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid JSON" })),
            );
        }
    };

    // 3. Handle URL verification challenge.
    if let Some(challenge_resp) = feishu_bot.handle_challenge(&payload) {
        info!("Feishu webhook: responded to URL verification challenge");
        return (StatusCode::OK, Json(challenge_resp));
    }

    // 4. Parse the event.
    let message = match feishu_bot.parse_event(&payload) {
        Ok(msg) => msg,
        Err(y_bot::BotError::UnsupportedEvent(evt_type)) => {
            // Non-message events (bot added, reactions, etc.) — acknowledge silently.
            info!(event_type = %evt_type, "Feishu webhook: ignoring non-message event");
            return (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "ignored" })),
            );
        }
        Err(e) => {
            warn!(error = %e, "Feishu webhook: failed to parse event");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Parse error: {e}") })),
            );
        }
    };

    // 5. Spawn async processing (Feishu requires <3s response).
    let container = Arc::clone(&state.container);
    let bot = Arc::clone(feishu_bot);
    tokio::spawn(async move {
        if let Err(e) = BotService::handle_message(&container, bot.as_ref(), message).await {
            error!(error = %e, "Feishu bot: message handling failed");
        }
    });

    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

// ---------------------------------------------------------------------------
// Discord webhook handler
// ---------------------------------------------------------------------------

/// `POST /api/v1/bots/discord/webhook` — receive Discord Interactions Endpoint callbacks.
///
/// Discord requires the endpoint to respond to PING interactions with a PONG
/// and validates Ed25519 signatures on every request.
async fn discord_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(ref discord_bot) = state.discord_bot else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Discord bot not configured" })),
        );
    };

    // Convert headers to HashMap<String, String> (lowercase keys).
    let header_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|val| (k.as_str().to_lowercase(), val.to_string()))
        })
        .collect();

    // 1. Verify Ed25519 signature.
    if let Err(e) = discord_bot.verify_signature(&header_map, &body) {
        warn!(error = %e, "Discord webhook: signature verification failed");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid signature" })),
        );
    }

    // 2. Parse JSON body.
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "Discord webhook: invalid JSON body");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid JSON" })),
            );
        }
    };

    // 3. Handle PING challenge (Interaction type 1).
    if let Some(pong_resp) = discord_bot.handle_challenge(&payload) {
        info!("Discord webhook: responded to PING interaction");
        return (StatusCode::OK, Json(pong_resp));
    }

    // 4. Parse the event.
    let message = match discord_bot.parse_event(&payload) {
        Ok(msg) => msg,
        Err(y_bot::BotError::UnsupportedEvent(evt_type)) => {
            info!(event_type = %evt_type, "Discord webhook: ignoring unsupported event");
            return (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "ignored" })),
            );
        }
        Err(e) => {
            warn!(error = %e, "Discord webhook: failed to parse event");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Parse error: {e}") })),
            );
        }
    };

    // 5. Spawn async processing.
    let container = Arc::clone(&state.container);
    let bot = Arc::clone(discord_bot);
    tokio::spawn(async move {
        if let Err(e) = BotService::handle_message(&container, bot.as_ref(), message).await {
            error!(error = %e, "Discord bot: message handling failed");
        }
    });

    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Bot webhook route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/bots/feishu/webhook", post(feishu_webhook))
        .route("/api/v1/bots/discord/webhook", post(discord_webhook))
}
