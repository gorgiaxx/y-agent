//! Bearer token authentication middleware.
//!
//! Validates bearer tokens against a configured token. Used to secure y-web
//! when exposed over the network.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;
use crate::state::AppState;

/// Middleware that validates bearer token authentication.
///
/// If `AppState.auth_token` is `Some(token)`, requests must include either an
/// `Authorization: Bearer <token>` header or a `?token=<token>` query
/// parameter for `EventSource` clients. Returns 401 Unauthorized on mismatch or
/// missing token.
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

    let Some(token) = request_token(&request) else {
        return ApiError::Unauthorized("Missing authorization token".to_string()).into_response();
    };

    if token != expected_token.as_str() {
        return ApiError::Unauthorized("Invalid token".to_string()).into_response();
    }

    next.run(request).await
}

fn request_token(request: &Request) -> Option<String> {
    request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.strip_prefix("Bearer ").unwrap_or(value).to_string())
        .or_else(|| query_token(request.uri().query().unwrap_or_default()))
}

fn query_token(query: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == "token" && !value.is_empty()).then(|| value.to_string())
    })
}

#[cfg(test)]
mod tests {
    use axum::body::Body;

    use super::*;

    fn request(uri: &str) -> Request {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    #[test]
    fn test_request_token_accepts_bearer_header() {
        let request = Request::builder()
            .uri("/api/v1/status")
            .header(axum::http::header::AUTHORIZATION, "Bearer secret")
            .body(Body::empty())
            .unwrap();

        assert_eq!(request_token(&request).as_deref(), Some("secret"));
    }

    #[test]
    fn test_request_token_accepts_eventsource_query_token() {
        let request = request("/api/v1/events?session_id=s1&token=secret");

        assert_eq!(request_token(&request).as_deref(), Some("secret"));
    }
}
