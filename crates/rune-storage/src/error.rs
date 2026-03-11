use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Object store error: {0}")]
    ObjectStore(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Migration error: {0}")]
    Migration(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Raft consensus error: {0}")]
    Raft(String),

    #[error("Unexpected write response")]
    UnexpectedResponse,
}
