use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("storage error: {0}")]
    Storage(#[from] rune_storage::StorageError),

    #[error("agent '{caller}' is not allowed to call '{callee}': no shared rune-network")]
    AccessDenied { caller: String, callee: String },
}
