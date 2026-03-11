use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("No healthy replica available for deployment {0}")]
    NoReplicaAvailable(String),

    #[error("Runtime error: {0}")]
    Runtime(#[from] rune_runtime::RuntimeError),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized".into()),
            Self::RateLimitExceeded => (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded".into()),
            Self::NoReplicaAvailable(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg.clone()),
            Self::Runtime(_) | Self::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into())
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
