//! Bearer token authentication middleware.
//!
//! Validates `Authorization: Bearer <token>` headers against a configured
//! token. Used to secure y-web when exposed over the network.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// Middleware that validates bearer token authentication.
///
/// If `AppState.auth_token` is `Some(token)`, all requests must include
/// `Authorization: Bearer <token>` header. Returns 401 Unauthorized on
/// mismatch or missing header.
///
/// If `AppState.auth_token` is `None`, all requests pass through without
/// authentication (local development mode).
pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(ref expected_token) = state.auth_token else {
        return next.run(request).await;
    };

    let auth_header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let Some(auth_value) = auth_header else {
        return (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response();
    };

    let token = auth_value
        .strip_prefix("Bearer ")
        .unwrap_or(auth_value);

    if token != expected_token.as_str() {
        return (StatusCode::UNAUTHORIZED, "Invalid token").into_response();
    }

    next.run(request).await
}
