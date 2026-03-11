use thiserror::Error;

#[derive(Debug, Error)]
pub enum A2aError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("JSON-RPC error {code}: {message}")]
    JsonRpc { code: i64, message: String },

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("agent discovery failed: {0}")]
    Discovery(String),

    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    #[error("timeout after {0}ms")]
    Timeout(u64),

    #[error("max call depth exceeded: {0}")]
    MaxDepthExceeded(u32),

    #[error("{0}")]
    Other(String),
}
