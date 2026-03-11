use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvError {
    #[error("missing required environment variable: {0}")]
    Missing(String),

    #[error("invalid value for {var}: {reason}")]
    Invalid { var: String, reason: String },

    #[error("configuration error: {0}")]
    Config(String),
}
