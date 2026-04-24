//! Unified API error type.
//!
//! All handler errors are converted to [`ApiError`], which implements
//! `IntoResponse` to produce consistent JSON error bodies.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

/// JSON error body returned to clients.
#[derive(Debug, Deserialize, Serialize)]
pub struct ErrorBody {
    /// Machine-readable error code (e.g. "`not_found`").
    pub error: String,
    /// Human-readable description.
    pub message: String,
}

/// API error enum — each variant maps to an HTTP status code.
#[derive(Debug)]
pub enum ApiError {
    /// 401 Unauthorized.
    Unauthorized(String),
    /// 404 Not Found.
    NotFound(String),
    /// 400 Bad Request.
    BadRequest(String),
    /// 500 Internal Server Error.
    Internal(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Unauthorized(msg) => write!(f, "unauthorized: {msg}"),
            ApiError::NotFound(msg) => write!(f, "not found: {msg}"),
            ApiError::BadRequest(msg) => write!(f, "bad request: {msg}"),
            ApiError::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match self {
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg),
        };

        let body = ErrorBody {
            error: error_code.to_string(),
            message,
        };

        (status, axum::Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ApiError::NotFound("session xyz".into());
        assert!(err.to_string().contains("session xyz"));

        let err = ApiError::BadRequest("missing field".into());
        assert!(err.to_string().contains("missing field"));

        let err = ApiError::Internal("db crash".into());
        assert!(err.to_string().contains("db crash"));
    }

    #[test]
    fn test_error_body_serialization() {
        let body = ErrorBody {
            error: "not_found".into(),
            message: "Session not found".into(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("not_found"));
        assert!(json.contains("Session not found"));
    }

    #[tokio::test]
    async fn test_unauthorized_response_is_json() {
        let response = ApiError::Unauthorized("missing token".into()).into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: ErrorBody = serde_json::from_slice(&body).unwrap();

        assert_eq!(body.error, "unauthorized");
        assert_eq!(body.message, "missing token");
    }
}
