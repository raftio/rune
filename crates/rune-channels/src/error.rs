use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChannelError {
    #[error("adapter not found: {0}")]
    AdapterNotFound(String),

    #[error("adapter already running: {0}")]
    AdapterAlreadyRunning(String),

    #[error("connection failed: {0}")]
    Connection(String),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("send failed: {0}")]
    Send(String),

    #[error("rate limited")]
    RateLimited,

    #[error("invalid signature")]
    InvalidSignature,

    #[error("invalid payload: {0}")]
    InvalidPayload(String),

    #[error("unsupported content type: {0}")]
    UnsupportedContent(String),

    #[error("adapter stopped")]
    Stopped,

    #[error("configuration error: {0}")]
    Config(String),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

impl From<String> for ChannelError {
    fn from(s: String) -> Self {
        Self::Other(s)
    }
}

impl From<&str> for ChannelError {
    fn from(s: &str) -> Self {
        Self::Other(s.to_string())
    }
}
