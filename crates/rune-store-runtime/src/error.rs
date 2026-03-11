use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeStoreError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("storage backend error: {0}")]
    Backend(String),
}
