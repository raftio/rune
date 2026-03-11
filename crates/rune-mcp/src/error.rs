use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("MCP protocol error {code}: {message}")]
    Protocol { code: i64, message: String },
    #[error("MCP server error: {0}")]
    Server(String),
    #[error("Tool not found: {0}")]
    ToolNotFound(String),
}
